//! Transaction message.

use {
    crate::{
        error::FixtureError,
        proto::{
            CompiledInstruction as ProtoCompiledInstruction,
            MessageAddressTableLookup as ProtoMessageAddressTableLookup,
            MessageHeader as ProtoMessageHeader, TransactionMessage as ProtoTransactionMessage,
        },
    },
    solana_hash::Hash,
    solana_pubkey::Pubkey,
    solana_sdk::{
        instruction::CompiledInstruction,
        message::{
            legacy,
            v0::{self, MessageAddressTableLookup},
            MessageHeader, VersionedMessage,
        },
    },
};

impl From<ProtoMessageHeader> for MessageHeader {
    fn from(value: ProtoMessageHeader) -> Self {
        Self {
            num_required_signatures: core::cmp::max(1, value.num_required_signatures as u8),
            num_readonly_signed_accounts: value.num_readonly_signed_accounts as u8,
            num_readonly_unsigned_accounts: value.num_readonly_unsigned_accounts as u8,
        }
    }
}

impl From<&MessageHeader> for ProtoMessageHeader {
    fn from(value: &MessageHeader) -> Self {
        Self {
            num_required_signatures: value.num_required_signatures as u32,
            num_readonly_signed_accounts: value.num_readonly_signed_accounts as u32,
            num_readonly_unsigned_accounts: value.num_readonly_unsigned_accounts as u32,
        }
    }
}

impl From<ProtoCompiledInstruction> for CompiledInstruction {
    fn from(value: ProtoCompiledInstruction) -> Self {
        Self {
            program_id_index: value.program_id_index as u8,
            accounts: value.accounts.iter().map(|i| *i as u8).collect(),
            data: value.data,
        }
    }
}

impl From<&CompiledInstruction> for ProtoCompiledInstruction {
    fn from(value: &CompiledInstruction) -> Self {
        Self {
            program_id_index: value.program_id_index as u32,
            accounts: value.accounts.iter().map(|i| *i as u32).collect(),
            data: value.data.clone(),
        }
    }
}

impl TryFrom<ProtoMessageAddressTableLookup> for MessageAddressTableLookup {
    type Error = FixtureError;

    fn try_from(value: ProtoMessageAddressTableLookup) -> Result<Self, Self::Error> {
        let account_key =
            Pubkey::try_from(value.account_key).map_err(FixtureError::InvalidPubkeyBytes)?;
        Ok(Self {
            account_key,
            writable_indexes: value.writable_indexes.iter().map(|i| *i as u8).collect(),
            readonly_indexes: value.readonly_indexes.iter().map(|i| *i as u8).collect(),
        })
    }
}

impl From<&MessageAddressTableLookup> for ProtoMessageAddressTableLookup {
    fn from(value: &MessageAddressTableLookup) -> Self {
        Self {
            account_key: value.account_key.to_bytes().to_vec(),
            writable_indexes: value.writable_indexes.iter().map(|i| *i as u32).collect(),
            readonly_indexes: value.readonly_indexes.iter().map(|i| *i as u32).collect(),
        }
    }
}

impl TryFrom<ProtoTransactionMessage> for VersionedMessage {
    type Error = FixtureError;

    fn try_from(value: ProtoTransactionMessage) -> Result<Self, Self::Error> {
        let header = if let Some(value_header) = value.header {
            MessageHeader::from(value_header)
        } else {
            MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            }
        };

        let account_keys = value
            .account_keys
            .into_iter()
            .map(|key| Pubkey::try_from(key).map_err(FixtureError::InvalidPubkeyBytes))
            .collect::<Result<Vec<Pubkey>, _>>()?;

        let recent_blockhash = if value.recent_blockhash.is_empty() {
            Hash::new_from_array([0u8; 32])
        } else {
            Hash::new(&value.recent_blockhash)
        };

        let instructions: Vec<CompiledInstruction> = value
            .instructions
            .into_iter()
            .map(CompiledInstruction::from)
            .collect();

        if value.is_legacy {
            let message = legacy::Message {
                header,
                account_keys,
                recent_blockhash,
                instructions,
            };
            Ok(VersionedMessage::Legacy(message))
        } else {
            let address_table_lookups = value
                .address_table_lookups
                .into_iter()
                .map(MessageAddressTableLookup::try_from)
                .collect::<Result<Vec<MessageAddressTableLookup>, _>>()?;

            let message = v0::Message {
                header,
                account_keys,
                recent_blockhash,
                instructions,
                address_table_lookups,
            };

            Ok(VersionedMessage::V0(message))
        }
    }
}

impl From<VersionedMessage> for ProtoTransactionMessage {
    fn from(value: VersionedMessage) -> Self {
        let account_keys = value
            .static_account_keys()
            .iter()
            .map(|key| key.to_bytes().to_vec())
            .collect();

        let instructions: Vec<ProtoCompiledInstruction> = value
            .instructions()
            .iter()
            .map(ProtoCompiledInstruction::from)
            .collect();

        let address_table_lookups: Vec<ProtoMessageAddressTableLookup> = value
            .address_table_lookups()
            .map(|lookups| {
                lookups
                    .iter()
                    .map(ProtoMessageAddressTableLookup::from)
                    .collect()
            })
            .unwrap_or_default();

        Self {
            is_legacy: value.address_table_lookups().is_some(),
            header: Some(value.header().into()),
            account_keys,
            recent_blockhash: value.recent_blockhash().to_bytes().to_vec(),
            instructions,
            address_table_lookups,
        }
    }
}
