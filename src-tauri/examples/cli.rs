// Classement en ligne de commande : cargo run --example cli -- <source>... <destination>
use flash::organizer::{organize, NameFormat};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage : cli <source>... <destination>");
        std::process::exit(1);
    }
    let (sources, dest) = args.split_at(args.len() - 1);
    match organize(sources, &dest[0], false, &NameFormat::default(), |p| {
        if p.phase == "apply" && p.done % 25 == 0 {
            eprintln!("  {}/{}", p.done, p.total);
        }
    }) {
        Ok(s) => {
            println!(
                "classés: {} · doublons: {} · renumérotés: {} · dates corrigées: {}",
                s.imported, s.duplicates, s.renamed, s.updated
            );
            if !s.uncertain.is_empty() {
                println!("dates incertaines ({}):", s.uncertain.len());
                for n in &s.uncertain {
                    println!("  ? {n}");
                }
            }
            for e in &s.errors {
                println!("  ! {e}");
            }
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            std::process::exit(1);
        }
    }
}
