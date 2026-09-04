use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc, Timelike};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub const MONTHS_FR: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

const IMAGE_EXTS: [&str; 17] = [
    "jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "tif", "tiff", "bmp", "dng", "cr2",
    "cr3", "nef", "arw", "orf", "rw2",
];
const VIDEO_EXTS: [&str; 9] = ["mp4", "mov", "m4v", "3gp", "avi", "mkv", "webm", "mts", "m2ts"];
// Formats de la famille MP4/QuickTime dont on sait lire la date dans l'atome mvhd
const MP4_LIKE: [&str; 4] = ["mp4", "mov", "m4v", "3gp"];

const INDEX_NAME: &str = ".tri-photos-index.json";
// Zone de triage pour les fichiers sans date fiable (ni métadonnées, ni nom)
const REVIEW_DIR: &str = "à vérifier";

fn ext_of(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_image(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

fn is_media(ext: &str) -> bool {
    is_image(ext) || VIDEO_EXTS.contains(&ext)
}

fn plausible(ts: NaiveDateTime) -> bool {
    (1980..2200).contains(&ts.year())
}

#[derive(Clone, Copy, PartialEq)]
enum TsSource {
    Meta,
    Name,
    FsDate,
}

// Déduit la date du nom de fichier : IMG-20260410-WA0012, PXL_20260410_150312123,
// « WhatsApp Video 2026-04-10 at 14.30.22 », « Capture d'écran 2026-04-10 à 17.03.12 »…
// Sans heure dans le nom, on prend 12h00 (neutre dans la journée).
fn filename_timestamp(name: &str) -> Option<NaiveDateTime> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"(20\d{2})[-_.]?(\d{2})[-_.]?(\d{2})(?:[\s\-_.]*(?:at|à)?[\s\-_.]*(\d{1,2})[h\-_.:]?(\d{2})(?:[m\-_.:]?(\d{2}))?)?",
        )
        .unwrap()
    });
    for cap in re.captures_iter(name) {
        let (Ok(y), Ok(mo), Ok(da)) = (
            cap[1].parse::<i32>(),
            cap[2].parse::<u32>(),
            cap[3].parse::<u32>(),
        ) else {
            continue;
        };
        let Some(date) = chrono::NaiveDate::from_ymd_opt(y, mo, da) else {
            continue;
        };
        let (h, mi, s) = match (cap.get(4), cap.get(5)) {
            (Some(h), Some(mi)) => {
                let h: u32 = h.as_str().parse().unwrap_or(99);
                let mi: u32 = mi.as_str().parse().unwrap_or(99);
                let s: u32 = cap
                    .get(6)
                    .and_then(|s| s.as_str().parse().ok())
                    .unwrap_or(0);
                if h <= 23 && mi <= 59 && s <= 59 {
                    (h, mi, s)
                } else {
                    (12, 0, 0)
                }
            }
            _ => (12, 0, 0),
        };
        if let Some(ndt) = date.and_hms_opt(h, mi, s) {
            if plausible(ndt) {
                return Some(ndt);
            }
        }
    }
    None
}

#[derive(Clone, Serialize)]
pub struct Progress {
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub message: Option<String>,
}

#[derive(Serialize, Default)]
pub struct Summary {
    pub imported: usize,
    pub duplicates: usize,
    pub renamed: usize,
    pub updated: usize,
    pub uncertain: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone)]
struct Rec {
    ts: NaiveDateTime,
    size: u64,
    hash: String,
    original: String,
}

struct NewEntry {
    src: PathBuf,
    ext: String,
    ts: NaiveDateTime,
    size: u64,
    hash: String,
    source: TsSource,
}

enum Item {
    Existing { rel: String, rec: Rec, ext: String },
    New(NewEntry),
}

// À heure identique (dates déduites sans heure, rafales…), l'ordre suit le nom
// d'origine : les compteurs type WA0012/WA0013 restent dans le bon ordre.
fn item_key(it: &Item) -> (NaiveDateTime, String, String) {
    match it {
        Item::Existing { rec, .. } => (rec.ts, rec.original.clone(), rec.hash.clone()),
        Item::New(e) => (
            e.ts,
            e.src
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            e.hash.clone(),
        ),
    }
}

// ---------- Lecture des dates de prise de vue ----------

fn exif_timestamp(path: &Path) -> Option<NaiveDateTime> {
    let file = File::open(path).ok()?;
    let mut br = BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut br).ok()?;
    for tag in [
        exif::Tag::DateTimeOriginal,
        exif::Tag::DateTimeDigitized,
        exif::Tag::DateTime,
    ] {
        let Some(field) = exif.get_field(tag, exif::In::PRIMARY) else {
            continue;
        };
        let exif::Value::Ascii(ref vecs) = field.value else {
            continue;
        };
        let Some(bytes) = vecs.first() else { continue };
        let Ok(dt) = exif::DateTime::from_ascii(bytes) else {
            continue;
        };
        let ndt = chrono::NaiveDate::from_ymd_opt(dt.year as i32, dt.month as u32, dt.day as u32)
            .and_then(|d| d.and_hms_opt(dt.hour as u32, dt.minute as u32, dt.second as u32));
        if let Some(ndt) = ndt {
            if plausible(ndt) {
                return Some(ndt);
            }
        }
    }
    None
}

// Date de création dans l'atome moov/mvhd (époque QuickTime : 1er janvier 1904 UTC).
fn mp4_creation_time(path: &Path) -> Option<NaiveDateTime> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();

    fn find_atom(f: &mut File, start: u64, end: u64, atom: &[u8; 4]) -> Option<(u64, u64, u64)> {
        let mut off = start;
        let mut header = [0u8; 16];
        while off + 8 <= end {
            f.seek(SeekFrom::Start(off)).ok()?;
            f.read_exact(&mut header[..8]).ok()?;
            let mut len = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
            let mut hs = 8u64;
            if len == 1 {
                f.read_exact(&mut header[8..16]).ok()?;
                len = u64::from_be_bytes(header[8..16].try_into().unwrap());
                hs = 16;
            } else if len == 0 {
                len = end - off;
            }
            if len < hs {
                return None;
            }
            if &header[4..8] == atom {
                return Some((off, hs, len));
            }
            off = off.checked_add(len)?;
        }
        None
    }

    let (m_off, m_hs, m_len) = find_atom(&mut f, 0, size, b"moov")?;
    let (v_off, v_hs, _) = find_atom(&mut f, m_off + m_hs, m_off + m_len, b"mvhd")?;
    f.seek(SeekFrom::Start(v_off + v_hs)).ok()?;
    let mut buf = [0u8; 12];
    f.read_exact(&mut buf).ok()?;
    let version = buf[0];
    let secs: u64 = if version == 1 {
        u64::from_be_bytes(buf[4..12].try_into().unwrap())
    } else {
        u32::from_be_bytes(buf[4..8].try_into().unwrap()) as u64
    };
    if secs == 0 {
        return None;
    }
    let epoch = Utc.with_ymd_and_hms(1904, 1, 1, 0, 0, 0).single()?;
    let dt = epoch.checked_add_signed(chrono::Duration::seconds(i64::try_from(secs).ok()?))?;
    let ndt = dt.with_timezone(&Local).naive_local();
    plausible(ndt).then_some(ndt)
}

fn fs_timestamp(md: &Metadata) -> NaiveDateTime {
    let mut best: Option<NaiveDateTime> = None;
    for t in [md.modified().ok(), md.created().ok()].into_iter().flatten() {
        let ndt = DateTime::<Local>::from(t).naive_local();
        if plausible(ndt) && best.map_or(true, |b| ndt < b) {
            best = Some(ndt);
        }
    }
    best.unwrap_or_else(|| Local::now().naive_local())
}

fn timestamp_for(path: &Path, ext: &str, md: &Metadata) -> (NaiveDateTime, TsSource) {
    let name_ts = path
        .file_name()
        .and_then(|n| filename_timestamp(&n.to_string_lossy()));
    if is_image(ext) {
        if let Some(ts) = exif_timestamp(path) {
            return (ts, TsSource::Meta);
        }
    } else if MP4_LIKE.contains(&ext) {
        if let Some(ts) = mp4_creation_time(path) {
            // Certains transferts (iCloud, messageries…) réécrivent la date
            // interne au moment de l'export. Une vidéo ne peut pas avoir été
            // capturée APRÈS la dernière modification du fichier : si la date
            // interne est nettement postérieure, elle ment.
            let fs_ts = fs_timestamp(md);
            if ts <= fs_ts + chrono::Duration::hours(24) {
                return (ts, TsSource::Meta);
            }
            if let Some(nts) = name_ts {
                return (nts, TsSource::Name);
            }
            return (fs_ts, TsSource::Meta);
        }
    }
    if let Some(ts) = name_ts {
        return (ts, TsSource::Name);
    }
    (fs_timestamp(md), TsSource::FsDate)
}

// ---------- Empreinte anti-doublons ----------
// sha1 des premiers et derniers 256 Ko + la taille : suffisant pour détecter
// les doublons sans lire des Go de vidéo.

fn hash_key(path: &Path, size: u64) -> std::io::Result<String> {
    const CHUNK: u64 = 256 * 1024;
    let mut f = File::open(path)?;
    let mut hasher = Sha1::new();
    let n = CHUNK.min(size) as usize;
    if n > 0 {
        let mut buf = vec![0u8; n];
        f.read_exact(&mut buf)?;
        hasher.update(&buf);
        if size > CHUNK {
            f.seek(SeekFrom::Start(size - CHUNK))?;
            f.read_exact(&mut buf)?;
            hasher.update(&buf);
        }
    }
    hasher.update(size.to_string().as_bytes());
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

// ---------- Nommage ----------

const MOIS_ABREGES: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.",
    "nov.", "déc.",
];

/// Format choisi par l'utilisateur : dossiers parents, éléments du nom de
/// fichier (dans l'ordre), mot-clé libre, style d'écriture par élément
/// (« avril » / « avr » / « 04 »…) et séparateur du nom. Si deux fichiers
/// obtiennent le même nom, le numéro est ajouté d'office (unicité garantie).
#[derive(Serialize, Deserialize, Clone)]
pub struct NameFormat {
    pub elements: Vec<String>,
    #[serde(default = "dossiers_par_defaut")]
    pub dossiers: Vec<String>,
    pub motcle: String,
    #[serde(default)]
    pub styles: HashMap<String, String>,
    #[serde(default = "separateur_par_defaut")]
    pub separateur: String,
}

fn separateur_par_defaut() -> String {
    " ".into()
}

impl NameFormat {
    fn style(&self, element: &str, defaut: &'static str) -> &str {
        self.styles.get(element).map(String::as_str).unwrap_or(defaut)
    }

    fn separateur_sur(&self) -> &str {
        match self.separateur.as_str() {
            "." | "-" | "_" => &self.separateur,
            _ => " ",
        }
    }
}

fn dossiers_par_defaut() -> Vec<String> {
    vec!["annee".into(), "mois".into()]
}

impl Default for NameFormat {
    fn default() -> Self {
        NameFormat {
            elements: vec!["jour".into(), "mois".into(), "numero".into()],
            dossiers: dossiers_par_defaut(),
            motcle: String::new(),
            styles: HashMap::new(),
            separateur: separateur_par_defaut(),
        }
    }
}

fn morceau(ts: &NaiveDateTime, element: &str, fmt: &NameFormat, n: usize) -> Option<String> {
    match element {
        "motcle" => {
            let propre: String = fmt
                .motcle
                .trim()
                .chars()
                .filter(|c| *c != '/' && *c != ':' && !c.is_control())
                .collect();
            if propre.is_empty() {
                None
            } else {
                Some(propre)
            }
        }
        "jour" => Some(match fmt.style("jour", "5") {
            "05" => format!("{:02}", ts.day()),
            _ => ts.day().to_string(),
        }),
        "mois" => Some(match fmt.style("mois", "avril") {
            "avr" => MOIS_ABREGES[ts.month0() as usize].to_string(),
            "04" => format!("{:02}", ts.month()),
            "4" => ts.month().to_string(),
            _ => MONTHS_FR[ts.month0() as usize].to_string(),
        }),
        "annee" => Some(match fmt.style("annee", "2026") {
            "26" => format!("{:02}", ts.year().rem_euclid(100)),
            _ => ts.year().to_string(),
        }),
        "heure" => Some(format!("{:02}h{:02}", ts.hour(), ts.minute())),
        "numero" => Some(n.to_string()),
        _ => None,
    }
}

/// Un segment de dossier correspond-il au jeton du format ?
fn segment_correspond(seg: &str, token: &str, fmt: &NameFormat) -> bool {
    match token {
        "annee" => match fmt.style("annee", "2026") {
            "26" => seg.len() == 2 && seg.chars().all(|c| c.is_ascii_digit()),
            _ => seg.len() == 4 && seg.chars().all(|c| c.is_ascii_digit()),
        },
        "mois" => match fmt.style("mois", "avril") {
            "avr" => MOIS_ABREGES.iter().any(|m| m.trim_end_matches('.') == seg),
            "04" | "4" => {
                !seg.is_empty()
                    && seg.len() <= 2
                    && seg.chars().all(|c| c.is_ascii_digit())
                    && seg.parse::<u32>().is_ok_and(|v| (1..=12).contains(&v))
            }
            _ => MONTHS_FR.contains(&seg),
        },
        "jour" => !seg.is_empty() && seg.len() <= 2 && seg.chars().all(|c| c.is_ascii_digit()),
        "heure" => seg.len() == 5 && seg.as_bytes().get(2) == Some(&b'h'),
        "motcle" => {
            let propre: String = fmt
                .motcle
                .trim()
                .chars()
                .filter(|c| *c != '/' && *c != ':' && !c.is_control())
                .collect();
            !propre.is_empty() && seg == propre
        }
        _ => false,
    }
}

/// Un chemin relatif suit-il la structure de dossiers du format courant ?
fn suit_structure(rel: &Path, fmt: &NameFormat) -> bool {
    let jetons: Vec<&String> = fmt.dossiers.iter().filter(|d| d.as_str() != "numero").collect();
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    // autant de dossiers que de jetons, plus le nom de fichier
    if comps.len() != jetons.len() + 1 {
        return false;
    }
    jetons
        .iter()
        .zip(&comps)
        .all(|(token, seg)| segment_correspond(seg, token, fmt))
}

fn target_rel(
    ts: &NaiveDateTime,
    n: usize,
    ext: &str,
    fmt: &NameFormat,
    force_numero: bool,
) -> String {
    // Un dossier ne finit jamais par un point (« avr. » -> « avr ») : les
    // dossiers à point final sont interdits sous Windows et pièges partout.
    let mut chemin: Vec<String> = fmt
        .dossiers
        .iter()
        .filter(|d| d.as_str() != "numero")
        .filter_map(|d| morceau(ts, d, fmt, n))
        .map(|seg| seg.trim_end_matches('.').to_string())
        .filter(|seg| !seg.is_empty())
        .collect();
    let mut nom: Vec<String> = fmt
        .elements
        .iter()
        .filter_map(|e| morceau(ts, e, fmt, n))
        .collect();
    if nom.is_empty() {
        nom.push(ts.day().to_string());
        nom.push(MONTHS_FR[ts.month0() as usize].to_string());
    }
    if force_numero && !fmt.elements.iter().any(|e| e == "numero") {
        nom.push(n.to_string());
    }
    // Jamais de point final avant l'extension (« 10 avr. » + .jpg -> « 10 avr.jpg »)
    let nom_joint = nom.join(fmt.separateur_sur());
    let nom_propre = nom_joint.trim_end_matches(['.', ' ']);
    chemin.push(format!("{}.{}", nom_propre, ext));
    chemin.join("/")
}

// ---------- Index ----------

#[derive(Serialize, Deserialize)]
struct IndexRec {
    ts: i64,
    size: u64,
    hash: String,
    original: String,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    files: BTreeMap<String, IndexRec>,
}

fn load_index(dest: &Path) -> BTreeMap<String, Rec> {
    let mut index = BTreeMap::new();
    let Ok(raw) = fs::read_to_string(dest.join(INDEX_NAME)) else {
        return index;
    };
    let Ok(data) = serde_json::from_str::<IndexFile>(&raw) else {
        return index;
    };
    for (rel, r) in data.files {
        if let Some(ts) = DateTime::from_timestamp_millis(r.ts).map(|d| d.naive_utc()) {
            index.insert(
                rel,
                Rec {
                    ts,
                    size: r.size,
                    hash: r.hash,
                    original: r.original,
                },
            );
        }
    }
    index
}

fn save_index(dest: &Path, index: &BTreeMap<String, Rec>) -> std::io::Result<()> {
    let files: BTreeMap<String, IndexRec> = index
        .iter()
        .map(|(rel, r)| {
            (
                rel.clone(),
                IndexRec {
                    ts: r.ts.and_utc().timestamp_millis(),
                    size: r.size,
                    hash: r.hash.clone(),
                    original: r.original.clone(),
                },
            )
        })
        .collect();
    let json = serde_json::to_string_pretty(&IndexFile { version: 1, files })?;
    let tmp = dest.join(format!("{INDEX_NAME}.tmp"));
    fs::write(&tmp, json)?;
    fs::rename(&tmp, dest.join(INDEX_NAME))
}

// Adopte les fichiers présents dans Année/mois mais absents de l'index
// (index supprimé, fichiers ajoutés à la main…) — l'index n'est qu'un cache.
fn adopt_orphans(dest: &Path, index: &mut BTreeMap<String, Rec>, fmt: &NameFormat) {
    // Les fichiers de « à vérifier » restent indexés (anti-doublons)
    if let Ok(files) = fs::read_dir(dest.join(REVIEW_DIR)) {
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !f.path().is_file() || !is_media(&ext_of(&f.path())) {
                continue;
            }
            let rel = format!("{REVIEW_DIR}/{name}");
            if index.contains_key(&rel) {
                continue;
            }
            let Ok(md) = f.metadata() else { continue };
            let Ok(hash) = hash_key(&f.path(), md.len()) else {
                continue;
            };
            index.insert(
                rel,
                Rec {
                    ts: fs_timestamp(&md),
                    size: md.len(),
                    hash,
                    original: name,
                },
            );
        }
    }
    // Le reste de la bibliothèque : seuls les fichiers qui suivent la
    // structure de dossiers CHOISIE sont adoptés — un dossier en vrac posé
    // dans la destination reste classable.
    let review_dir = dest.join(REVIEW_DIR);
    let walker = walkdir::WalkDir::new(dest)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            !e.file_name().to_string_lossy().starts_with('.') && e.path() != review_dir
        });
    for f in walker.flatten() {
        if !f.file_type().is_file() {
            continue;
        }
        let ext = ext_of(f.path());
        if !is_media(&ext) {
            continue;
        }
        let Ok(rel_path) = f.path().strip_prefix(dest) else {
            continue;
        };
        if !suit_structure(rel_path, fmt) {
            continue;
        }
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        if index.contains_key(&rel) {
            continue;
        }
        let Ok(md) = f.metadata() else { continue };
        let (ts, _) = timestamp_for(f.path(), &ext, &md);
        let Ok(hash) = hash_key(f.path(), md.len()) else {
            continue;
        };
        let original = f.file_name().to_string_lossy().into_owned();
        index.insert(
            rel,
            Rec {
                ts,
                size: md.len(),
                hash,
                original,
            },
        );
    }
}

// Un chemin relatif à la destination est « géré » s'il est sous Année/mois/.
// Seuls ces fichiers sont exclus du scan : on peut donc classer sur place un
// dossier en vrac posé dans la destination.
fn is_managed_rel(rel: &Path) -> bool {
    let mut comps = rel.components();
    if let Some(first) = rel.components().next() {
        if first.as_os_str().to_string_lossy() == REVIEW_DIR {
            return true;
        }
    }
    let year_ok = comps.next().is_some_and(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.len() == 4 && s.chars().all(|ch| ch.is_ascii_digit())
    });
    let month_ok = comps.next().is_some_and(|c| {
        let s = c.as_os_str().to_string_lossy();
        MONTHS_FR.contains(&s.as_ref())
    });
    year_ok && month_ok && comps.next().is_some()
}

// ---------- Traitement principal ----------

pub fn organize<F: Fn(Progress) + Sync>(
    sources: &[String],
    dest: &str,
    move_files: bool,
    fmt: &NameFormat,
    progress: F,
) -> Result<Summary, String> {
    let dest = PathBuf::from(dest);
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let mut summary = Summary::default();

    // 1. Scan des sources
    progress(Progress {
        phase: "scan".into(),
        done: 0,
        total: 0,
        message: Some("Analyse des dossiers…".into()),
    });
    let mut src_files: Vec<PathBuf> = Vec::new();
    for s in sources {
        let p = PathBuf::from(s);
        let Ok(md) = fs::metadata(&p) else { continue };
        if md.is_dir() {
            let walker = walkdir::WalkDir::new(&p).into_iter().filter_entry(|e| {
                e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
            });
            for entry in walker.flatten() {
                if entry.file_type().is_file() && is_media(&ext_of(entry.path())) {
                    src_files.push(entry.into_path());
                }
            }
        } else if md.is_file() && is_media(&ext_of(&p)) {
            src_files.push(p);
        }
    }
    // 2. Index existant + adoption des fichiers non indexés — chargé AVANT le
    // filtre : un fichier déjà géré, quelle que soit l'arborescence choisie,
    // est connu de l'index.
    let mut index = load_index(&dest);
    index.retain(|rel, _| dest.join(rel).exists());
    adopt_orphans(&dest, &mut index, fmt);

    let dest_canon = dest.canonicalize().unwrap_or_else(|_| dest.clone());
    let mut seen = HashSet::new();
    let candidates: Vec<PathBuf> = src_files
        .into_iter()
        .filter(|f| {
            let abs = f.canonicalize().unwrap_or_else(|_| f.clone());
            if let Ok(rel) = abs.strip_prefix(&dest_canon) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if index.contains_key(&rel_str) || suit_structure(rel, fmt) || is_managed_rel(rel) {
                    return false;
                }
            }
            seen.insert(abs)
        })
        .collect();

    // 3. Dates + empreintes des nouveaux fichiers (en parallèle)
    let done = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::<String>::new());
    let total = candidates.len();
    let analyzed: Vec<Option<NewEntry>> = candidates
        .par_iter()
        .map(|src| {
            let r = (|| -> Result<NewEntry, String> {
                let md = fs::metadata(src).map_err(|e| e.to_string())?;
                let ext = ext_of(src);
                let (ts, source) = timestamp_for(src, &ext, &md);
                let hash = hash_key(src, md.len()).map_err(|e| e.to_string())?;
                Ok(NewEntry {
                    src: src.clone(),
                    ext,
                    ts,
                    size: md.len(),
                    hash,
                    source,
                })
            })();
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress(Progress {
                phase: "analyze".into(),
                done: d,
                total,
                message: None,
            });
            match r {
                Ok(e) => Some(e),
                Err(e) => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("{} : {}", src.display(), e));
                    None
                }
            }
        })
        .collect();
    summary.errors.extend(errors.into_inner().unwrap());

    // 4. Doublons (contre l'existant et au sein du lot). Si un doublon apporte
    //    une date fiable (métadonnées ou nom) différente de celle enregistrée,
    //    on corrige la date du fichier déjà classé au lieu de juste l'ignorer.
    let mut known: HashMap<(u64, String), Option<String>> = index
        .iter()
        .map(|(rel, r)| ((r.size, r.hash.clone()), Some(rel.clone())))
        .collect();
    let mut healed: HashSet<String> = HashSet::new();
    let mut fresh = Vec::new();
    let mut uncertain_new = Vec::new();
    for e in analyzed.into_iter().flatten() {
        let key = (e.size, e.hash.clone());
        match known.get(&key).cloned() {
            None => {
                known.insert(key, None);
                if e.source == TsSource::FsDate {
                    uncertain_new.push(e);
                } else {
                    fresh.push(e);
                }
            }
            Some(existing_rel) => {
                summary.duplicates += 1;
                if let Some(rel) = existing_rel {
                    if e.source != TsSource::FsDate {
                        if let Some(rec) = index.get_mut(&rel) {
                            if rec.ts != e.ts {
                                rec.ts = e.ts;
                                healed.insert(e.hash.clone());
                                summary.updated += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // 5. Regroupement par jour, tri par heure, numérotation
    let mut by_day: BTreeMap<(i32, u32, u32), Vec<Item>> = BTreeMap::new();
    let mut final_index: BTreeMap<String, Rec> = BTreeMap::new();
    let review_prefix = format!("{REVIEW_DIR}/");
    for (rel, rec) in &index {
        // Un fichier en zone de triage n'entre dans la numérotation que si un
        // doublon vient de lui apporter une date fiable
        if rel.starts_with(&review_prefix) && !healed.contains(&rec.hash) {
            final_index.insert(rel.clone(), rec.clone());
            continue;
        }
        let k = (rec.ts.year(), rec.ts.month(), rec.ts.day());
        let ext = ext_of(Path::new(rel));
        by_day.entry(k).or_default().push(Item::Existing {
            rel: rel.clone(),
            rec: rec.clone(),
            ext,
        });
    }
    for e in fresh {
        let k = (e.ts.year(), e.ts.month(), e.ts.day());
        by_day.entry(k).or_default().push(Item::New(e));
    }

    struct RenamePlan {
        from: String,
        to: String,
    }
    struct CopyPlan {
        src: PathBuf,
        to: String,
        rec: Rec,
        review: bool,
    }
    let mut renames: Vec<RenamePlan> = Vec::new();
    let mut copies: Vec<CopyPlan> = Vec::new();
    let mut pris: HashSet<String> = HashSet::new();
    for (_k, mut items) in by_day {
        items.sort_by(|a, b| item_key(a).cmp(&item_key(b)));
        for (i, item) in items.into_iter().enumerate() {
            let n = i + 1;
            match item {
                Item::Existing { rel, rec, ext } => {
                    let mut essai = n;
                    let mut to = target_rel(&rec.ts, essai, &ext, fmt, false);
                    while pris.contains(&to) {
                        essai += 1;
                        to = target_rel(&rec.ts, essai, &ext, fmt, true);
                    }
                    pris.insert(to.clone());
                    if rel != to {
                        renames.push(RenamePlan {
                            from: rel,
                            to: to.clone(),
                        });
                    }
                    final_index.insert(to, rec);
                }
                Item::New(e) => {
                    let mut essai = n;
                    let mut to = target_rel(&e.ts, essai, &e.ext, fmt, false);
                    while pris.contains(&to) {
                        essai += 1;
                        to = target_rel(&e.ts, essai, &e.ext, fmt, true);
                    }
                    pris.insert(to.clone());
                    let original = e
                        .src
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    copies.push(CopyPlan {
                        src: e.src,
                        to,
                        rec: Rec {
                            ts: e.ts,
                            size: e.size,
                            hash: e.hash,
                            original,
                        },
                        review: false,
                    });
                }
            }
        }
    }

    // Les fichiers sans date fiable partent en zone de triage, sous leur nom
    // d'origine (suffixe « 2 », « 3 »… en cas d'homonymes)
    let mut planned_review: HashSet<String> = HashSet::new();
    for e in uncertain_new {
        let original = e
            .src
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (stem, ext_dot) = match original.rfind('.') {
            Some(i) => (&original[..i], &original[i..]),
            None => (original.as_str(), ""),
        };
        let mut to = format!("{REVIEW_DIR}/{original}");
        let mut n = 2;
        while final_index.contains_key(&to) || planned_review.contains(&to) || dest.join(&to).exists()
        {
            to = format!("{REVIEW_DIR}/{stem} {n}{ext_dot}");
            n += 1;
        }
        planned_review.insert(to.clone());
        copies.push(CopyPlan {
            src: e.src,
            to,
            rec: Rec {
                ts: e.ts,
                size: e.size,
                hash: e.hash,
                original,
            },
            review: true,
        });
    }

    // 6. Renommages en deux phases (évite les collisions quand
    //    « 10 avril 2 » devient « 10 avril 3 » pendant que 2 est repris)
    let total_apply = renames.len() + copies.len();
    let mut applied = 0usize;
    let mut staged: Vec<(PathBuf, String)> = Vec::new();
    let mut old_dirs: HashSet<PathBuf> = HashSet::new();
    for (i, r) in renames.iter().enumerate() {
        if let Some(parent) = dest.join(&r.from).parent() {
            old_dirs.insert(parent.to_path_buf());
        }
        let tmp = dest.join(format!(".tri-photos-tmp-{i}"));
        match fs::rename(dest.join(&r.from), &tmp) {
            Ok(_) => staged.push((tmp, r.to.clone())),
            Err(e) => {
                summary.errors.push(format!("Renommage de {} : {}", r.from, e));
                final_index.remove(&r.to);
            }
        }
    }
    for (tmp, to) in staged {
        let abs = dest.join(&to);
        if let Some(parent) = abs.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::rename(&tmp, &abs) {
            Ok(_) => summary.renamed += 1,
            Err(e) => {
                summary.errors.push(format!("Renommage vers {to} : {e}"));
                final_index.remove(&to);
            }
        }
        applied += 1;
        progress(Progress {
            phase: "apply".into(),
            done: applied,
            total: total_apply,
            message: None,
        });
    }

    // 7. Copie (ou déplacement) des nouveaux fichiers
    for c in copies {
        let abs = dest.join(&c.to);
        let res = (|| -> Result<(), String> {
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if abs.exists() {
                return Err("un fichier existe déjà à cet emplacement".into());
            }
            if move_files {
                if fs::rename(&c.src, &abs).is_err() {
                    fs::copy(&c.src, &abs).map_err(|e| e.to_string())?;
                    fs::remove_file(&c.src).map_err(|e| e.to_string())?;
                }
            } else {
                fs::copy(&c.src, &abs).map_err(|e| e.to_string())?;
            }
            // Cale la date du fichier sur la prise de vue (tri Finder cohérent)
            if let Some(local) = Local.from_local_datetime(&c.rec.ts).earliest() {
                let ft = filetime::FileTime::from_system_time(local.into());
                let _ = filetime::set_file_times(&abs, ft, ft);
            }
            Ok(())
        })();
        match res {
            Ok(_) => {
                if c.review {
                    summary.uncertain.push(c.rec.original.clone());
                } else {
                    summary.imported += 1;
                }
                final_index.insert(c.to.clone(), c.rec);
            }
            Err(e) => {
                let name = c
                    .src
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                summary.errors.push(format!("{name} : {e}"));
            }
        }
        applied += 1;
        progress(Progress {
            phase: "apply".into(),
            done: applied,
            total: total_apply,
            message: None,
        });
    }

    // Recale la date de fichier des éléments dont la date a été corrigée
    for (rel, rec) in &final_index {
        if healed.contains(&rec.hash) {
            if let Some(local) = Local.from_local_datetime(&rec.ts).earliest() {
                let ft = filetime::FileTime::from_system_time(local.into());
                let _ = filetime::set_file_times(dest.join(rel), ft, ft);
            }
        }
    }

    // Supprime les dossiers mois/année devenus vides après renumérotation
    for dir in &old_dirs {
        if fs::remove_dir(dir).is_ok() {
            if let Some(year) = dir.parent() {
                if year != dest {
                    let _ = fs::remove_dir(year);
                }
            }
        }
    }

    save_index(&dest, &final_index).map_err(|e| e.to_string())?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_mtime(p: &Path, ts: NaiveDateTime) {
        let st: std::time::SystemTime = Local.from_local_datetime(&ts).earliest().unwrap().into();
        let ft = filetime::FileTime::from_system_time(st);
        filetime::set_file_times(p, ft, ft).unwrap();
    }

    fn mk(p: &Path, content: &[u8], ts: NaiveDateTime) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
        set_mtime(p, ts);
    }

    fn d(y: i32, mo: u32, da: u32, h: u32, mi: u32) -> NaiveDateTime {
        ds(y, mo, da, h, mi, 0)
    }

    fn ds(y: i32, mo: u32, da: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(y, mo, da)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
    }

    #[test]
    fn deduit_la_date_du_nom_de_fichier() {
        assert_eq!(
            filename_timestamp("IMG-20260410-WA0012.jpg"),
            Some(d(2026, 4, 10, 12, 0))
        );
        assert_eq!(
            filename_timestamp("WhatsApp Video 2026-04-10 at 14.30.22.mp4"),
            Some(ds(2026, 4, 10, 14, 30, 22))
        );
        assert_eq!(
            filename_timestamp("Capture d'écran 2026-04-10 à 17.03.12.png"),
            Some(ds(2026, 4, 10, 17, 3, 12))
        );
        assert_eq!(
            filename_timestamp("PXL_20260410_150312123.mp4"),
            Some(ds(2026, 4, 10, 15, 3, 12))
        );
        assert_eq!(
            filename_timestamp("Screenshot_20260410-170312.png"),
            Some(ds(2026, 4, 10, 17, 3, 12))
        );
        assert_eq!(
            filename_timestamp("20260410_170312.jpg"),
            Some(ds(2026, 4, 10, 17, 3, 12))
        );
        assert_eq!(filename_timestamp("DSC_1234.jpg"), None);
        assert_eq!(filename_timestamp("IMG_9999.HEIC"), None);
    }

    #[test]
    fn lit_la_date_de_creation_mvhd_dun_mp4() {
        let root = std::env::temp_dir().join(format!("tri-photos-mvhd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let utc = Utc.with_ymd_and_hms(2026, 4, 10, 15, 0, 0).unwrap();
        let epoch = Utc.with_ymd_and_hms(1904, 1, 1, 0, 0, 0).unwrap();
        let secs = (utc - epoch).num_seconds() as u32;
        let mut mvhd: Vec<u8> = Vec::new();
        mvhd.extend_from_slice(&20u32.to_be_bytes());
        mvhd.extend_from_slice(b"mvhd");
        mvhd.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        mvhd.extend_from_slice(&secs.to_be_bytes()); // creation_time
        mvhd.extend_from_slice(&secs.to_be_bytes()); // modification_time
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"ftypisom");
        data.extend_from_slice(&[0, 0, 2, 0]);
        data.extend_from_slice(&((8 + mvhd.len()) as u32).to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&mvhd);
        let p = root.join("v.mp4");
        fs::write(&p, &data).unwrap();
        let expected = utc.with_timezone(&Local).naive_local();
        assert_eq!(mp4_creation_time(&p), Some(expected));
        let _ = fs::remove_dir_all(&root);
    }

    // Cas réel : un dossier en vrac posé DANS la destination doit se classer
    // sur place ; seuls les fichiers déjà sous Année/mois sont exclus du scan.
    #[test]
    fn classe_un_dossier_situe_dans_la_destination() {
        let root = std::env::temp_dir().join(format!("tri-photos-inplace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dest = root.join("dest");
        let vrac = dest.join("vrac");
        mk(&vrac.join("20260410_090000.jpg"), b"photoA", d(2026, 4, 10, 9, 0));
        let s1 = organize(
            &[vrac.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s1.imported, 1, "erreurs: {:?}", s1.errors);
        assert!(dest.join("2026/avril/10 avril 1.jpg").exists());

        // Re-dépôt du même dossier : doublon reconnu, rien en double
        let s2 = organize(
            &[vrac.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s2.imported, 0);
        assert_eq!(s2.duplicates, 1);

        // Dépôt de la racine entière : les fichiers déjà classés sont exclus
        let s3 = organize(
            &[dest.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s3.imported, 0);
        assert_eq!(s3.duplicates, 1); // seul vrac/a.jpg est re-scanné
        let _ = fs::remove_dir_all(&root);
    }

    // Cas réel : un transfert iCloud/messagerie réécrit la date interne du MP4
    // au moment de l'export ; les dates du fichier, plus anciennes, sont les bonnes.
    #[test]
    fn ignore_une_date_interne_reecrite_apres_coup() {
        let root = std::env::temp_dir().join(format!("tri-photos-liar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // mvhd prétend « maintenant » (récent), le fichier date d'avril 2025
        let lying_utc = Utc.with_ymd_and_hms(2026, 8, 3, 13, 21, 16).unwrap();
        let epoch = Utc.with_ymd_and_hms(1904, 1, 1, 0, 0, 0).unwrap();
        let secs = (lying_utc - epoch).num_seconds() as u32;
        let mut mvhd: Vec<u8> = Vec::new();
        mvhd.extend_from_slice(&20u32.to_be_bytes());
        mvhd.extend_from_slice(b"mvhd");
        mvhd.extend_from_slice(&[0, 0, 0, 0]);
        mvhd.extend_from_slice(&secs.to_be_bytes());
        mvhd.extend_from_slice(&secs.to_be_bytes());
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"ftypisom");
        data.extend_from_slice(&[0, 0, 2, 0]);
        data.extend_from_slice(&((8 + mvhd.len()) as u32).to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&mvhd);
        let p = root.join("225BFAB1.MP4");
        fs::write(&p, &data).unwrap();
        let real = d(2025, 4, 30, 19, 21);
        set_mtime(&p, real);
        let md = fs::metadata(&p).unwrap();
        let (ts, _) = timestamp_for(&p, "mp4", &md);
        assert_eq!(ts, real, "la date interne réécrite doit être ignorée");

        // …mais une date interne cohérente (≤ dates fichier) reste prioritaire
        let honest = root.join("honest.mp4");
        fs::write(&honest, &data).unwrap();
        set_mtime(&honest, d(2026, 9, 1, 10, 0)); // fichier copié après la capture
        let md2 = fs::metadata(&honest).unwrap();
        let (ts2, _) = timestamp_for(&honest, "mp4", &md2);
        assert_eq!(ts2, lying_utc.with_timezone(&Local).naive_local());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zone_de_triage_puis_sortie_quand_la_date_arrive() {
        let root = std::env::temp_dir().join(format!("tri-photos-heal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dest = root.join("dest");

        // Import initial : ni métadonnées ni date dans le nom → zone de triage
        let src1 = root.join("src1");
        mk(&src1.join("video.mkv"), b"filmX", d(2025, 1, 2, 3, 4));
        let s1 = organize(
            &[src1.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s1.imported, 0, "erreurs: {:?}", s1.errors);
        assert_eq!(s1.uncertain, vec!["video.mkv".to_string()]);
        assert!(dest.join("à vérifier/video.mkv").exists());

        // Re-dépôt du même dossier : doublon reconnu, pas de deuxième copie
        let s1b = organize(
            &[src1.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s1b.duplicates, 1);
        assert_eq!(s1b.uncertain.len(), 0);

        // Re-dépôt du même contenu avec la vraie date dans le nom :
        // le fichier sort de la zone de triage et rejoint l'archive
        let src2 = root.join("src2");
        mk(&src2.join("VID-20260410-WA0007.mkv"), b"filmX", d(2025, 1, 2, 3, 4));
        let s2 = organize(
            &[src2.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s2.imported, 0);
        assert_eq!(s2.duplicates, 1);
        assert_eq!(s2.updated, 1, "erreurs: {:?}", s2.errors);
        assert_eq!(s2.renamed, 1);
        assert!(dest.join("2026/avril/10 avril 1.mkv").exists());
        assert!(
            !dest.join("à vérifier").exists(),
            "zone de triage vide non nettoyée"
        );

        let _ = fs::remove_dir_all(&root);
    }

    // Deux inconnus homonymes ne s'écrasent pas dans la zone de triage
    #[test]
    fn triage_gere_les_homonymes() {
        let root = std::env::temp_dir().join(format!("tri-photos-homon-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dest = root.join("dest");
        let src = root.join("src");
        mk(&src.join("un/photo.jpg"), b"contenuUN", d(2025, 1, 2, 3, 4));
        mk(&src.join("deux/photo.jpg"), b"contenuDEUX", d(2025, 3, 4, 5, 6));
        let s = organize(
            &[src.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s.uncertain.len(), 2, "erreurs: {:?}", s.errors);
        assert!(dest.join("à vérifier/photo.jpg").exists());
        assert!(dest.join("à vérifier/photo 2.jpg").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn organise_puis_renumerote_les_lots_suivants() {
        let root = std::env::temp_dir().join(format!("tri-photos-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src1 = root.join("src1");
        let dest = root.join("dest");

        // Lot 1 : deux fichiers le 10 avril (9h et 17h), un le 5 mars
        // (dates dans les noms : source fiable, pas de passage en zone de triage)
        mk(&src1.join("IMG_20260410_090000.jpg"), b"photoA", d(2026, 4, 10, 9, 0));
        mk(&src1.join("VID_20260410_170000.mp4"), b"videoB", d(2026, 4, 10, 17, 0));
        mk(&src1.join("sub/20260305_120000.jpg"), b"photoC", d(2026, 3, 5, 12, 0));
        let s1 = organize(
            &[src1.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s1.imported, 3, "erreurs: {:?}", s1.errors);
        assert!(dest.join("2026/avril/10 avril 1.jpg").exists());
        assert!(dest.join("2026/avril/10 avril 2.mp4").exists());
        assert!(dest.join("2026/mars/5 mars 1.jpg").exists());

        // Lot 2 : une photo à midi s'intercale, un doublon est ignoré
        let src2 = root.join("src2");
        mk(&src2.join("20260410_120000.jpg"), b"photoD", d(2026, 4, 10, 12, 0));
        mk(&src2.join("dup_20260410_090000.jpg"), b"photoA", d(2026, 4, 10, 9, 0));
        let s2 = organize(
            &[src2.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s2.imported, 1, "erreurs: {:?}", s2.errors);
        assert_eq!(s2.duplicates, 1);
        assert_eq!(s2.renamed, 1); // la vidéo de 17h passe de 2 à 3
        assert_eq!(
            fs::read(dest.join("2026/avril/10 avril 1.jpg")).unwrap(),
            b"photoA"
        );
        assert_eq!(
            fs::read(dest.join("2026/avril/10 avril 2.jpg")).unwrap(),
            b"photoD"
        );
        assert_eq!(
            fs::read(dest.join("2026/avril/10 avril 3.mp4")).unwrap(),
            b"videoB"
        );

        // Lot 3 : l'index supprimé n'empêche rien (adoption + re-dérivation)
        fs::remove_file(dest.join(INDEX_NAME)).unwrap();
        let src3 = root.join("src3");
        mk(&src3.join("20260410_073000.jpg"), b"photoE", d(2026, 4, 10, 7, 30));
        let s3 = organize(
            &[src3.to_string_lossy().into_owned()],
            dest.to_str().unwrap(),
            false,
            &NameFormat::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s3.imported, 1, "erreurs: {:?}", s3.errors);
        assert_eq!(
            fs::read(dest.join("2026/avril/10 avril 1.jpg")).unwrap(),
            b"photoE"
        );
        assert_eq!(
            fs::read(dest.join("2026/avril/10 avril 2.jpg")).unwrap(),
            b"photoA"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
