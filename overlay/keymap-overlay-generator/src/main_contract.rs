use anyhow::Result;
use keymap_overlay_generator::contract::schema_json;
use std::io::{self, Write as _};

fn main() -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(schema_json()?.as_bytes())?;
    stdout.write_all(b"\n")?;
    Ok(())
}
