# Tri Photos

App bureau (Tauri 2 + Rust) qui classe automatiquement photos et vidéos par glisser-déposer.

## Ce que ça fait

Tu glisses un ou plusieurs dossiers (ou fichiers) dans la fenêtre, et tout est classé dans le dossier de destination :

```
Destination/
  2026/
    mars/
      5 mars 1.jpg
    avril/
      10 avril 1.jpg    ← photo de 9h
      10 avril 2.jpg    ← photo de 12h
      10 avril 3.mp4    ← vidéo de 17h
```

- **Date de prise de vue** : EXIF pour les photos (JPEG, HEIC, PNG, RAW…), métadonnées QuickTime/MP4 pour les vidéos — recoupées avec les dates du fichier pour détecter les métadonnées réécrites par un transfert (iCloud, messageries…). À défaut, la date est lue dans le nom du fichier (`VID-20250410-WA0012`, `PXL_20250410_…`, captures d'écran…), sinon dates du fichier.
- **Auto-correction** : redéposer un fichier déjà classé avec une meilleure date le reclasse au bon endroit.
- **Zone de triage** : un fichier sans aucune date fiable (ni métadonnées, ni nom) part dans `à vérifier/` sous son nom d'origine au lieu de fausser la numérotation ; il en sort automatiquement si un re-dépôt apporte sa vraie date.
- **Numérotation par heure** dans la journée. Si un nouveau lot contient une photo plus ancienne qu'une déjà classée, tout le jour est **renuméroté automatiquement**.
- **Doublons ignorés** (empreinte du contenu) : redéposer un dossier déjà traité n'importe rien en double.
- **Copie par défaut** (les originaux ne bougent pas) ; case à cocher pour déplacer.
- Un index `.tri-photos-index.json` dans la destination accélère les traitements suivants. S'il est supprimé, l'app re-scanne et se répare toute seule.

## Lancer en développement

```bash
npm install
npm run dev
```

## Construire l'app (.app / .dmg)

```bash
npm run build
```

Le binaire est dans `src-tauri/target/release/bundle/`.

## Tests

```bash
cd src-tauri && cargo test
```

## CLI (sans interface)

```bash
cd src-tauri && cargo run --example cli -- "/chemin/source" "/chemin/destination"
```

## Site vitrine

Landing Next.js dans `site/`, avec la `.dmg` téléchargeable dans `site/public/downloads/`.

```bash
cd site && npm install && npm run dev
```
