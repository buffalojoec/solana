use solana_svm_feature_set::SVMFeatureSet;

pub struct Scenario {
    pub track_memory: bool,
    pub nesting_levels: Vec<u8>,
    pub account_data_len: usize,
    pub direct_mapping: bool,
}

impl Scenario {
    pub fn frames(&self) -> usize {
        self.nesting_levels
            .iter()
            .map(|level| 1 + *level as usize)
            .sum()
    }

    pub fn feature_set(&self) -> SVMFeatureSet {
        SVMFeatureSet {
            account_data_direct_mapping: self.direct_mapping,
            ..SVMFeatureSet::all_enabled()
        }
    }
}
