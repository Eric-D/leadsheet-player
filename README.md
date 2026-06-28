# leadsheet

Lecteur web de *lead sheets* (grilles d'accords + mélodie) en **Rust / WebAssembly**.
Lit les fichiers **Band-in-a-Box** (`.MGU` / `.SGU`) et **MIDI** (`.mid`), affiche la
grille d'accords, la partition et la tablature, joue la mélodie + un accompagnement
généré, et tourne **hors-ligne** (PWA installable).

## Structure (workspace Cargo)

| Crate | Rôle |
|-------|------|
| [`leadsheet/`](leadsheet) | Lib pure Rust : modèle commun + lecteurs de formats (`biab`, `midi`) + **encodeur** `.MGU` + styles d'accompagnement (`style`) + arrangeur (`arrange`). Aucune dépendance web. |
| [`web/`](web) | L'application (egui/eframe + Web Audio + bibliothèque IndexedDB + PWA), dépend de `leadsheet`. |

```rust
let song   = leadsheet::parse(bytes)?;            // auto-détecte BiaB ou MIDI
let style  = leadsheet::style::Style::default();  // ou Style::import(ron)
let events = leadsheet::arrange::arrange(&song, &style);
```

## Développement

```sh
# Application (serveur de dev avec rechargement à chaud)
cd web && trunk serve            # http://localhost:8080

# Tests de la lib (décodeur + encodeur, aller-retour)
cargo test -p leadsheet

# Générer la démo + les fixtures originales (.MGU)
cargo run -p leadsheet --example gen
```

Prérequis : `rustup target add wasm32-unknown-unknown` et [Trunk](https://trunkrs.dev).

## Déploiement

Push sur `main` → GitHub Actions build l'app et la publie sur **GitHub Pages**
(voir [`.github/workflows/deploy.yml`](.github/workflows/deploy.yml)).
À activer une fois : *Settings → Pages → Source : GitHub Actions*.

## Contenu

Le dépôt ne contient **aucune chanson sous droit d'auteur** : la démo intégrée et les
fixtures de test sont des progressions **originales**, générées par l'encodeur. Les
fichiers personnels de test vivent dans `private/` (ignoré par git).

## Licence

[MIT](LICENSE).
