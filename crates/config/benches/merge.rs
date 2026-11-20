//! Pure strategy benchmark for OPT-016: the plan's Value-tree merge — one
//! parse per layer, one tree merge, one typed decode — from pre-read TOML
//! text, so the filesystem does not dominate the measurement. Measured
//! slower than the shipped direct decode (benches/load.rs) and kept as the
//! measurement record.

use reasonix_config::Config;
use serde::Deserialize;
use toml::Table;

pub fn decode_merged_layers(texts: &[&str]) -> Config {
    let mut merged = Table::new();
    for (index, text) in texts.iter().enumerate() {
        let table: Table = toml::from_str(text).expect("bench fixture parses");
        let mut table = table;
        if index > 0 {
            // load_project.go userGlobals, as Config::load applies it.
            table.remove("serve");
            table.remove("storage");
        }
        merge_into(&mut merged, &table);
    }
    Config::deserialize(toml::Value::Table(merged)).expect("bench fixture decodes")
}

fn merge_into(base: &mut Table, over: &Table) {
    for (key, value) in over {
        if let toml::Value::Table(nested) = value
            && let Some(toml::Value::Table(existing)) = base.get_mut(key)
        {
            merge_into(existing, nested);
            continue;
        }
        base.insert(key.clone(), value.clone());
    }
}
