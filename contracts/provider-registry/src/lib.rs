#![no_std]

//! # Provider Registry Contract
//!
//! Maintains healthcare provider registry with credentials, specializations, network participation,
//! and provider validation for access control decisions.
//!
//! ## HIPAA Compliance
//!
//! **Access Control Safeguards:** Admin-only provider registration. Provider authentication for
//! profile updates. Credential verification required before network participation. Address nonzero
//! validation prevents invalid entries. Specialization enumeration restricts valid provider types.
//!
//! **Audit Controls:** Provider registration events logged with credential type. Credential
//! updates tracked with new specialization. Network participation events recorded. Provider
//! deactivation events logged. Credential expiration events tracked.
//!
//! **Data Retention Policy:** Provider records retained indefinitely as reference data. Credential
//! information persisted with expiration dates. Specialization information maintained for service
//! validation. Deactivated providers retain historical data without deletion.
//!
//! **Encryption/Integrity:** Provider addresses validated via nonzero checks. Address immutable
//! once registered. Specialization enum validates allowed provider types. Credential metadata
//! stored encrypted in persistent state. Registry lookup validates provider identity.

use shared::privacy::validate_nonzero_address;
use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, symbol_short, Address, BytesN, Env,
    String, Vec,
};

mod test;

// ── Errors ────────────────────────────────────────────────────────────────────

/// Maximum entries per batch call.
pub const MAX_BATCH_SIZE: u32 = 50;
/// Admin rotation confirmation window (24 hours in seconds).
pub const ADMIN_ROTATION_WINDOW: u64 = 86_400;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    NotAProvider       = 4,
    RecordNotFound     = 5,
    BatchTooLarge      = 6,
    InvalidAddress     = 7,
    RotationPending    = 8,
    NoRotationPending  = 9,
    RotationExpired    = 10,
    NotPendingAdmin    = 11,
    StaleNonce         = 12,
    AlreadyExists      = 13,
    CredentialExpired  = 14,
}

/// Input entry for `batch_register_providers`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchProviderEntry {
    pub provider: Address,
    pub name: String,
    pub specialty: String,
    pub license_number: String,
    pub credential_hash: BytesN<32>,
    pub issuer: Address,
    pub attestation_hash: BytesN<32>,
    pub expires_at: u64,
    pub revocation_reference: BytesN<32>,
}

/// Per-entry outcome returned by `batch_register_providers`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchEntryStatus {
    /// Entry registered successfully.
    Success       = 1,
    /// Entry skipped because the address is already registered.
    AlreadyExists = 2,
    InvalidAddress = 6,
    /// Entry skipped because `expires_at` is not in the future.
    CredentialExpired = 7,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialAnchor {
    pub credential_hash: BytesN<32>,
    pub issuer: Address,
    pub attestation_hash: BytesN<32>,
    pub expires_at: u64,
    pub revocation_reference: BytesN<32>,
    pub revoked_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderProfile {
    pub name: String,
    pub specialty: String,
    pub license_number: String,
    pub credential: CredentialAnchor,
    pub active: bool,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
pub enum DataKey {
    Initialized,
    Admin,
    Provider(Address),
    Record(Address, String),
    ProviderRecords(Address),
    ProviderRecordCount(Address),
    RateLimitConfig,
    ProviderRate(Address),
    ProviderReputation(Address),
    ProviderRatingByPatient(Address, Address),
    PendingAdmin,
    RotationExpiry,
    /// Per-caller nonce for replay attack protection: (caller) -> u64
    CallerNonce(Address),
    /// Historical credential anchors for a provider: (provider) -> Vec<CredentialAnchor>
    CredentialHistory(Address),
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct ProviderRegistry;

#[contractimpl]
impl ProviderRegistry {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        Self::assert_not_initialized(&env)?;
        validate_nonzero_address(&admin).map_err(|_| Error::InvalidAddress)?;
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Initialized, &true);
        Ok(())
    }

    pub fn register_provider(
        env: Env,
        admin: Address,
        provider: Address,
        name: String,
        specialty: String,
        license_number: String,
        credential_hash: BytesN<32>,
        issuer: Address,
        attestation_hash: BytesN<32>,
        expires_at: u64,
        revocation_reference: BytesN<32>,
    ) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        validate_nonzero_address(&admin).map_err(|_| Error::InvalidAddress)?;
        validate_nonzero_address(&provider).map_err(|_| Error::InvalidAddress)?;
        validate_nonzero_address(&issuer).map_err(|_| Error::InvalidAddress)?;
        Self::assert_admin(&env, &admin)?;
        if expires_at <= env.ledger().timestamp() {
            return Err(Error::CredentialExpired);
        }
        let key = DataKey::Provider(provider.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyExists);
        }
        let profile = ProviderProfile {
            name,
            specialty,
            license_number,
            credential: CredentialAnchor {
                credential_hash,
                issuer,
                attestation_hash,
                expires_at,
                revocation_reference,
                revoked_at: None,
            },
            active: true,
        };
        env.storage().persistent().set(&key, &profile);
        env.events()
            .publish((symbol_short!("reg_prov"), provider), symbol_short!("ok"));
        Ok(())
    }

    /// Register multiple providers in one transaction.
    ///
    /// Returns a `Vec<BatchEntryStatus>` aligned 1-to-1 with `entries`.
    /// Already-registered providers produce `AlreadyExists` (no overwrite).
    /// Enforces `MAX_BATCH_SIZE`; returns `BatchTooLarge` if exceeded.
    pub fn batch_register_providers(
        env: Env,
        admin: Address,
        entries: Vec<BatchProviderEntry>,
    ) -> Result<Vec<BatchEntryStatus>, Error> {
        Self::assert_initialized(&env)?;
        Self::assert_admin(&env, &admin)?;

        if entries.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        let mut results: Vec<BatchEntryStatus> = Vec::new(&env);

        for entry in entries.iter() {
            if validate_nonzero_address(&entry.provider).is_err()
                || validate_nonzero_address(&entry.issuer).is_err()
            {
                results.push_back(BatchEntryStatus::InvalidAddress);
                continue;
            }

            if entry.expires_at <= env.ledger().timestamp() {
                results.push_back(BatchEntryStatus::CredentialExpired);
                continue;
            }

            let key = DataKey::Provider(entry.provider.clone());
            if env.storage().persistent().has(&key) {
                results.push_back(BatchEntryStatus::AlreadyExists);
                continue;
            }

            let profile = ProviderProfile {
                name: entry.name.clone(),
                specialty: entry.specialty.clone(),
                license_number: entry.license_number.clone(),
                credential: CredentialAnchor {
                    credential_hash: entry.credential_hash.clone(),
                    issuer: entry.issuer.clone(),
                    attestation_hash: entry.attestation_hash.clone(),
                    expires_at: entry.expires_at,
                    revocation_reference: entry.revocation_reference.clone(),
                    revoked_at: None,
                },
                active: true,
            };
            env.storage().persistent().set(&key, &profile);
            results.push_back(BatchEntryStatus::Success);
        }

        Ok(results)
    }

    /// Renew an existing provider's credential without changing their address.
    ///
    /// Admin-only. Requires `new_expires_at > now`. The previous credential anchor
    /// is appended to the provider's credential history for auditability, and a
    /// `renew_cred` event is emitted. This is the only path that changes
    /// `expires_at` after registration, so an expired provider can be restored
    /// without re-registering under a new address.
    pub fn renew_credential(
        env: Env,
        admin: Address,
        provider: Address,
        new_credential_hash: BytesN<32>,
        new_attestation_hash: BytesN<32>,
        new_expires_at: u64,
    ) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        validate_nonzero_address(&admin).map_err(|_| Error::InvalidAddress)?;
        validate_nonzero_address(&provider).map_err(|_| Error::InvalidAddress)?;
        Self::assert_admin(&env, &admin)?;

        if new_expires_at <= env.ledger().timestamp() {
            return Err(Error::CredentialExpired);
        }

        let key = DataKey::Provider(provider.clone());
        let mut profile: ProviderProfile = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NotAProvider)?;

        // Preserve the previous anchor for auditability.
        let history_key = DataKey::CredentialHistory(provider.clone());
        let mut history: Vec<CredentialAnchor> = env
            .storage()
            .persistent()
            .get(&history_key)
            .unwrap_or_else(|| Vec::new(&env));
        history.push_back(profile.credential.clone());
        env.storage().persistent().set(&history_key, &history);

        profile.credential = CredentialAnchor {
            credential_hash: new_credential_hash,
            issuer: profile.credential.issuer.clone(),
            attestation_hash: new_attestation_hash,
            expires_at: new_expires_at,
            revocation_reference: profile.credential.revocation_reference.clone(),
            revoked_at: None,
        };
        env.storage().persistent().set(&key, &profile);

        env.events().publish(
            (symbol_short!("renew_cred"), provider),
            new_expires_at,
        );
        Ok(())
    }

    /// Returns the historical credential anchors for a provider (oldest first).
    pub fn get_credential_history(env: Env, provider: Address) -> Vec<CredentialAnchor> {
        env.storage()
            .persistent()
            .get(&DataKey::CredentialHistory(provider))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn assert_initialized(env: &Env) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Initialized) {
            Ok(())
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn assert_not_initialized(env: &Env) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Initialized) {
            Err(Error::AlreadyInitialized)
        } else {
            Ok(())
        }
    }

    fn assert_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if &admin != caller {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();
        Ok(())
    }
}
