//! Merkle tree-based data structure designed to store receipts (traces) from
//! processed SVM transactions.
//!
//! The SVM is considered to be a state transition function (STF). If you apply
//! a directive D(n) to some state S(n), you should deterministically obtain
//! new state S(n+1). Because the STF is deterministic, it can generate proofs.
//!
//! ### State
//!
//! The SVM state is simply the collective account state of all accounts
//! involved in the transaction batch.
//!
//! > Caveat: This is only an implementation detail right now. The SVM API will
//!   load all accounts for an entire batch, so it's trivial to capture a state
//!   snapshot for an entire batch. However, certain constraints would need to
//!   be imposed in order to capture such a snapshot for a block.
//!
//! Account state is hashed using ... (TBD)
//!
//! ### Directives
//!
//! SVM's STF directive can be considered the combination of the transaction
//! as well as input parameters for configuring the SVM environment (such as
//! feature set, rent and fee details, and compute budget).
//!
//! As such, an STF directive here is the collective hash of all of these
//! inputs. This is done by ... (TBD)
//!
//! ### Trace
//!
//! Even though state and directives are all you need to generate proofs for
//! STF, a user may also desire to verify the result of their transaction in,
//! say, a Solana block. This may include not only account state updates but
//! also status code, log messages, and return data.
//!
//! On the Solana L1, if a transaction has been "processed" by SVM, it has been
//! written to a block. Processed transactions can be fees-only (invalid
//! transaction where fee was still deducted) or executed. If a fee was
//! deducted, that means account state changed, and that would be captured by
//! STF proofs as described above.
//!
//! In the case of a transaction that was executed, a _trace_ must be computed,
//! storing:
//!
//! * Compute units consumed.
//! * Log messages.
//! * Return data.
//! * Status (code).
//!
//! Insert details about algorithms, yuck ... (TBD)

pub mod joe;
pub mod receipt;
pub mod stf;
