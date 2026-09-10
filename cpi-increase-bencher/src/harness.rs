use {
    crate::{
        measure::{Measurement, measure},
        scenario::Scenario,
    },
    solana_program_runtime::loaded_programs::ProgramCacheForTxBatch,
    solana_pubkey::Pubkey,
    solana_svm::conformance::programs::{
        add_program_to_program_cache, new_program_cache_with_builtins,
    },
    solana_svm_feature_set::SVMFeatureSet,
};

pub struct Harness {
    pub(crate) program_id: Pubkey,
    pub(crate) elf: Vec<u8>,
    pub(crate) program_cache: ProgramCacheForTxBatch,
}

impl Harness {
    pub fn new(elf: &[u8], feature_set: &SVMFeatureSet) -> Self {
        let program_id = Pubkey::new_unique();
        let mut program_cache = new_program_cache_with_builtins(1);
        add_program_to_program_cache(
            &mut program_cache,
            &program_id,
            &solana_sdk_ids::bpf_loader::id(),
            elf,
            feature_set,
        );
        Self {
            program_id,
            elf: elf.to_vec(),
            program_cache,
        }
    }

    pub fn measure(&mut self, scenario: &Scenario) -> Measurement {
        measure(self, scenario)
    }
}
