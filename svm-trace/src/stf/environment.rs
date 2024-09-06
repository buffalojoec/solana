use {
    solana_sdk::{feature_set::FeatureSet, fee::FeeStructure, keccak::Hasher, rent::Rent},
    solana_svm_rent_collector::svm_rent_collector::SVMRentCollector,
};

fn hash_feature_set(hasher: &mut Hasher, feature_set: &FeatureSet) {
    feature_set
        .active
        .iter()
        .map(|(feature, _)| feature)
        .chain(feature_set.inactive.iter())
        .for_each(|feature| {
            hasher.hash(feature.as_ref());
        });
}

fn hash_fee_structure(hasher: &mut Hasher, fee_structure: &FeeStructure) {
    hasher.hash(&fee_structure.lamports_per_signature.to_le_bytes());
    hasher.hash(&fee_structure.lamports_per_write_lock.to_le_bytes());
    // `compute_fee_bins` skipped for now.
}

fn hash_rent(hasher: &mut Hasher, rent: &Rent) {
    hasher.hash(&rent.lamports_per_byte_year.to_le_bytes());
    hasher.hash(&rent.exemption_threshold.to_le_bytes());
    hasher.hash(&rent.burn_percent.to_le_bytes());
}

fn hash_rent_collector(hasher: &mut Hasher, rent_collector: &dyn SVMRentCollector) {
    hash_rent(hasher, &rent_collector.get_rent());
}

pub(crate) fn hash_environment(hasher: &mut Hasher, environment: &STFEnvironment) {
    hash_feature_set(hasher, environment.feature_set);
    environment.fee_structure.map(|fee_structure| {
        hash_fee_structure(hasher, fee_structure);
    });
    hasher.hash(&environment.lamports_per_signature.to_le_bytes());
    environment.rent_collector.map(|rent_collector| {
        hash_rent_collector(hasher, rent_collector);
    });
}

pub struct STFEnvironment<'a> {
    pub feature_set: &'a FeatureSet,
    pub fee_structure: Option<&'a FeeStructure>,
    pub lamports_per_signature: u64,
    pub rent_collector: Option<&'a dyn SVMRentCollector>,
}
