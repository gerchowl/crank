//! Export the desktop command registry to the committed TypeScript contract.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    crank_desktop::bindings().export(
        specta_typescript::Typescript::default(),
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "app/src/bindings.ts".into()),
    )?;
    Ok(())
}
