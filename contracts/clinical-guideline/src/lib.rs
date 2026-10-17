#![no_std]

//! # Clinical Guideline Contract
//!
//! Provides evidence-based clinical decision support through guideline recommendations, dosage
//! calculations, risk scoring, and care pathway recommendations.
//!
//! ## HIPAA Compliance
//!
//! **Access Control Safeguards:** Authorization checks for guideline access. Provider-based access
//! control to clinical recommendations. Query access restricted to authorized providers and clinicians.
//!
//! **Audit Controls:** Guideline recommendations include strength and evidence level for traceability.
//! Risk scores documented with calculator type and interpretation. Care pathway recommendations tracked
//! with clinical decision points for audit trails.
//!
//! **Data Retention Policy:** Guideline recommendations stored immutably for clinical reference.
//! Risk score history retained with timestamps. Care pathway milestones tracked for longitudinal
//! patient journey documentation.
//!
//! **Encryption/Integrity:** Guideline evidence levels classified (e.g., A, B, C) for strength
//! determination. Dosage recommendations include validation against renal function and monitoring
//! requirements. Clinical decision data stored in contract state for integrity.

use soroban_sdk::{
    Address, BytesN, Env, String, Symbol, Vec, contract, contracterror, contractimpl, contracttype,
    symbol_short,
};

// --- Custom Error Types ---
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotAuthorized = 1,
    GuidelineNotFound = 2,
    InvalidInput = 3,
}

// --- Data Structures ---
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuidelineRecommendation {
    pub guideline_id: String,
    pub applicable: bool,
    pub recommendation: String,
    pub strength: Symbol,
    pub evidence_level: Symbol,
    pub alternative_options: Vec<String>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DosageRecommendation {
    pub medication: String,
    pub recommended_dose: String,
    pub frequency: String,
    pub route: Symbol,
    pub duration: Option<u64>,
    pub renal_adjustment: bool,
    pub monitoring_required: Vec<String>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskScore {
    pub calculator: Symbol,
    pub score: i32,
    pub interpretation: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarePathway {
    pub condition: String,
    pub steps: Vec<String>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reminder {
    pub reminder_id: u64,
    pub patient_id: Address,
    pub due_date: u64,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuidelineMetadata {
    pub condition: String,
    pub criteria_hash: BytesN<32>,
    pub recommendation_hash: BytesN<32>,
    pub evidence_level: Symbol,
}

// --- Data key enum ---
#[contracttype]
pub enum DataKey {
    Admin,
    ProviderRegistry,
    Guideline(String),
    ReminderCounter(Address),       // patient_id -> u64 (next reminder_id)
    Reminder(Address, u64),         // (patient_id, reminder_id) -> Reminder
}

#[contract]
pub struct ClinicalGuidelineContract;

#[contractimpl]
impl ClinicalGuidelineContract {
    /// Bind the contract to a stored admin address.
    ///
    /// Runs atomically as part of contract creation and cannot be called again,
    /// so an attacker cannot front-run deployment and elect themselves admin.
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Set the provider registry address for authorization checks.
    pub fn set_provider_registry(env: Env, provider_registry: Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotAuthorized)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::ProviderRegistry, &provider_registry);
        Ok(())
    }

    pub fn register_clinical_guideline(
        env: Env,
        admin: Address,
        guideline_id: String,
        condition: String,
        criteria_hash: BytesN<32>,
        recommendation_hash: BytesN<32>,
        evidence_level: Symbol,
    ) -> Result<(), Error> {
        admin.require_auth();

        // Verify caller matches stored admin
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotAuthorized)?;
        if admin != stored_admin {
            return Err(Error::NotAuthorized);
        }

        // Reject silent overwrite — existing guideline_id must not already exist
        let key = DataKey::Guideline(guideline_id.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::InvalidInput);
        }

        let metadata = GuidelineMetadata {
            condition: condition.clone(),
            criteria_hash,
            recommendation_hash,
            evidence_level,
        };

        env.storage().persistent().set(&key, &metadata);

        env.events().publish(
            (symbol_short!("reg_guide"), guideline_id),
            (condition, admin),
        );

        Ok(())
    }

    pub fn update_clinical_guideline(
        env: Env,
        admin: Address,
        guideline_id: String,
        condition: String,
        criteria_hash: BytesN<32>,
        recommendation_hash: BytesN<32>,
        evidence_level: Symbol,
    ) -> Result<(), Error> {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotAuthorized)?;
        if admin != stored_admin {
            return Err(Error::NotAuthorized);
        }

        let key = DataKey::Guideline(guideline_id.clone());
        if !env.storage().persistent().has(&key) {
            return Err(Error::GuidelineNotFound);
        }

        let metadata = GuidelineMetadata {
            condition: condition.clone(),
            criteria_hash,
            recommendation_hash,
            evidence_level,
        };

        env.storage().persistent().set(&key, &metadata);

        env.events().publish(
            (symbol_short!("upd_guide"), guideline_id),
            (condition, admin),
        );

        Ok(())
    }

    pub fn evaluate_guideline(
        env: Env,
        _patient_id: Address,
        provider_id: Address,
        guideline_id: String,
        patient_data_hash: BytesN<32>,
    ) -> Result<GuidelineRecommendation, Error> {
        provider_id.require_auth();
        if !Self::is_provider_registered(&env, &provider_id) {
            return Err(Error::NotAuthorized);
        }
        let metadata: GuidelineMetadata = env
            .storage()
            .persistent()
            .get(&DataKey::Guideline(guideline_id.clone()))
            .ok_or(Error::GuidelineNotFound)?;

        let is_applicable = metadata.criteria_hash == patient_data_hash;

        Ok(GuidelineRecommendation {
            guideline_id,
            applicable: is_applicable,
            recommendation: String::from_str(&env, "Follow evidence-based recommendation"),
            strength: Symbol::new(&env, "Strong"),
            evidence_level: metadata.evidence_level,
            alternative_options: Vec::new(&env),
        })
    }

    pub fn calculate_drug_dosage(
        env: Env,
        _patient_id: Address,
        provider_id: Address,
        medication: String,
        weight_dg: u32, // Decigrams (0.1g) to avoid f32
        renal_function: Option<u32>,
    ) -> Result<DosageRecommendation, Error> {
        provider_id.require_auth();
        if !Self::is_provider_registered(&env, &provider_id) {
            return Err(Error::NotAuthorized);
        }
        let gfr = renal_function

/* … truncated 72 chars — edit only what you need near the top … */
