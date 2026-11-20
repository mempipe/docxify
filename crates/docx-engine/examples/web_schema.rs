//! Emit the serialized shapes used by the browser.
use docx_engine::{editing::Resolution, Command, Inspection, Splice, ValidationReport};
use schemars::{generate::SchemaSettings, JsonSchema};

#[derive(JsonSchema)]
#[allow(dead_code)]
struct WebApi {
    command: Command,
    inspection: Inspection<'static>,
    resolution: Resolution,
    splice: Splice,
    validation: ValidationReport,
}

fn main() {
    let schema = SchemaSettings::draft07()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<WebApi>();
    println!("{}", serde_json::to_string(&schema).unwrap());
}
