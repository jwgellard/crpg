use crpg_data::SourcePath;
use std::{collections::BTreeMap, fs, path::Path};

pub fn fixture_files() -> BTreeMap<SourcePath, Vec<u8>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/one_area_one_creature");
    [
        "campaign.json",
        "campaign.lock",
        "worlds/world.json",
        "areas/start/area.json",
        "areas/start/placements.json",
        "areas/start/triggers.json",
        "creatures/creature.json",
        "variables/campaign_state.json",
        "assets/assets.lock",
        "locale/en.json",
    ]
    .into_iter()
    .map(|path| {
        (
            path.parse().unwrap(),
            fs::read(root.join(path)).expect("required fixture file"),
        )
    })
    .collect()
}
