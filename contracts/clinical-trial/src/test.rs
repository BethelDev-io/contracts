use crate::{ClinicalTrialContractClient, CriteriaRule, DataFilters, Error};
use soroban_sdk::{symbol_short, testutils::Address as _, testutils::Events, Address, Bytes, BytesN, Env, String, Vec};
use soroban_sdk::xdr::ToXdr;

fn create_test_env() -> (Env, Address, Address, Address, ClinicalTrialContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let pi = Address::generate(&env);
    let patient = Address::generate(&env);

    let contract_id = env.register(crate::ClinicalTrialContract, (&admin, &pi));
    let client = ClinicalTrialContractClient::new(&env, &contract_id);

    (env, admin, pi, patient, client)
}

fn create_protocol_hash(env: &Env) -> BytesN<32> {
    let data = String::from_str(env, "protocol_v1");
    env.crypto().sha256(&data.into()).into()
}

fn make_rule(env: &Env, parameter: &str, value: &str) -> CriteriaRule {
    CriteriaRule {
        criteria_type: symbol_short!("demo"),
        parameter: String::from_str(env, parameter),
        operator: symbol_short!("eq"),
        value: String::from_str(env, value),
        mandatory: true,
    }
}

fn expected_claim_hash(
    env: &Env,
    trial_record_id: u64,
    patient_data_hash: &BytesN<32>,
    rule: &CriteriaRule,
) -> BytesN<32> {
    let mut payload = Bytes::new(env);
    payload.append(&Bytes::from_slice(env, b"trial-eligibility-v1"));
    payload.append(&Bytes::from_slice(env, &trial_record_id.to_be_bytes()));
    payload.append(&patient_data_hash.clone().into());
    payload.append(&rule.criteria_type.clone().to_xdr(env));
    payload.append(&rule.parameter.clone().to_xdr(env));
    payload.append(&rule.operator.clone().to_xdr(env));
    payload.append(&rule.value.clone().to_xdr(env));
    env.crypto().sha256(&payload).into()
}

#[test]
fn test_constructor_binds_admin() {
    let (env, admin, _, _, client) = create_test_env();

    // Successful registration confirms contract is initialized
    let trial_record_id = client.register_clinical_trial(
        &admin,
        &String::from_str(&env, "TRIAL001"),
        &String::from_str(&env, "Cancer Treatment Study"),
        &symbol_short!("phase2"),
        &create_protocol_hash(&env),
        &1000,
        &2000,
        &100,
        &String::from_str(&env, "IRB-2024-001"),
    );

    assert_eq!(trial_record_id, 0u64);
}

#[test]
fn test_register_clinical_trial() {
    let (env, _, pi, _, client) = create_test_env();

    let trial_record_id = client.register_clinical_trial(
        &pi,
        &String::from_str(&env, "TRIAL001"),
        &String::from_str(&env, "Diabetes Study"),
        &symbol_short!("phase3"),
        &create_protocol_hash(&env),
        &1000,
        &5000,
        &200,
        &String::from_str(&env, "IRB-2024-002"),
    );

    let trial_data = client.get_trial(&trial_record_id);
    assert_eq!(trial_data.trial_record_id, trial_record_id);
    assert_eq!(trial_data.principal_investigator, pi);
    assert_eq!(trial_data.enrollment_target, 200);
}

#[test]
fn test_invalid_study_phase() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_register_clinical_trial(
        &pi,
        &String::from_str(&env, "TRIAL001"),
        &String::from_str(&env, "Test Study"),
        &symbol_short!("invalid"),
        &create_protocol_hash(&env),
        &1000,
        &5000,
        &100,
        &String::from_str(&env, "IRB-2024-003"),
    );

    assert!(result.is_err());
}

#[test]
fn test_invalid_date_range() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_register_clinical_trial(
        &pi,
        &String::from_str(&env, "TRIAL001"),
        &String::from_str(&env, "Test Study"),
        &symbol_short!("phase1"),
        &create_protocol_hash(&env),
        &5000,
        &1000, // end before start
        &100,
        &String::from_str(&env, "IRB-2024-004"),
    );

    assert!(result.is_err());
}

#[test]
fn test_withdrawal_policy_enforces_data_retention() {
    let (env, _, pi, patient, client) = create_test_env();

    let trial_record_id = client.register_clinical_trial(
        &pi,
        &String::from_str(&env, "TRIAL001"),
        &String::from_str(&env, "Withdrawal Policy Study"),
        &symbol_short!("phase2"),
        &create_protocol_hash(&env),
        &1000,
        &2000,
        &100,
        &String::from_str(&env, "IRB-2024-007"),
    );

    let enrollment_id = client.enroll_participant(
        &trial_record_id,
        &patient,
        &symbol_short!("armA"),
        &1100,
        &create_protocol_hash(&env),
        &String::from_str(&env, "PATIENT001"),
    );

    client.withdraw_participant(
        &enrollment_id,
        &1200,
        &symbol_short!("consent"),
        &false,
    );

    let events = env.events().all();
    assert_eq!(events.len(), 5);

    let filters = DataFilters {
        include_withdrawn: true,
        study_arms: Vec::new(&env),
        date_range_start: None,
        date_range_end: None,
    };

    let export_hash = client.export_deidentified_data(&trial_record_id, &pi, &filters);
    let expected_hash = env
        .crypto()
        .sha256(&soroban_sdk::Bytes::from_slice(&env, &0u32.to_be_bytes()));
    assert_eq!(export_hash, expected_hash);

    let result = client.try_record_study_visit(
        &enrollment_id,
        &1u32,
        &1300,
        &symbol_short!("followup"),
        &create_protocol_hash(&env),
        &Vec::new(&env),
    );
    assert_eq!(result, Err(Ok(Error::WithdrawalRestricted)));

    let event_id = client.report_adverse_event(
        &enrollment_id,
        &symbol_short!("headache"),
        &symbol_short!("moderate"),
        &create_protocol_hash(&env),
        &1300,
        &Option::<u64>::None,
        &symbol_short!("possible"),
    );

    let adverse_event = client.get_adverse_event(&event_id, &pi);
    assert_eq!(adverse_event.enrollment_id, enrollment_id);
}

// ── #484: multi-site enrolment tests ─────────────────────────────────────────────

fn setup_trial_with_sites(
    env: &Env,
    client: &ClinicalTrialContractClient,
    pi: &Address,
) -> (u64, u64, u64, Address, Address) {
    let trial_id = client.register_clinical_trial(
        pi,
        &String::from_str(env, "MULTI-SITE-01"),
        &String::from_str(env, "Multi-Site Study"),
        &symbol_short!("phase2"),
        &create_protocol_hash(env),
        &1000,
        &9999,
        &200, // total cap
        &String::from_str(env, "IRB-2024-100"),
    );

    let coord_a = Address::generate(env);
    let coord_b = Address::generate(env);

    let site_a = client.add_site(&trial_id, pi, &coord_a, &50);
    let site_b = client.add_site(&trial_id, pi, &coord_b, &10);

    (trial_id, site_a, site_b, coord_a, coord_b)
}

#[test]
fn test_enrol_at_site_succeeds() {
    let (env, _, pi, patient, client) = create_test_env();
    let (trial_id, site_a, _, coord_a, _) = setup_trial_with_sites(&env, &client, &pi);

    let enrollment_id = client.enrol_participant_at_site(
        &trial_id,
        &site_a,
        &coord_a,
        &patient,
        &symbol_short!("armA"),
        &1100,
        &create_protocol_hash(&env),
        &String::from_str(&env, "P001"),
    );

    let enrollment = client.get_enrollment(&enrollment_id, &pi);
    assert_eq!(enrollment.site_id, Some(site_a));
    assert_eq!(enrollment.trial_record_id, trial_id);
}

// ── #848: core regulatory function coverage ──────────────────────────────────────

fn setup_regulatory_trial(
    env: &Env,
    client: &ClinicalTrialContractClient,
    pi: &Address,
) -> u64 {
    client.register_clinical_trial(
        pi,
        &String::from_str(env, "REG-848-01"),
        &String::from_str(env, "Regulatory Coverage Study"),
        &symbol_short!("phase2"),
        &create_protocol_hash(env),
        &1000,
        &9999,
        &100,
        &String::from_str(env, "IRB-2024-848"),
    )
}

#[test]
fn test_define_eligibility_criteria_success() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let rules = Vec::from_array(&env, [make_rule(&env, "age", "18")]);
    client.define_eligibility_criteria(&trial_id, &pi, &rules);

    let stored = client.get_eligibility_criteria(&trial_id);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored.get(0).unwrap().parameter, String::from_str(&env, "age"));
}

#[test]
fn test_define_eligibility_criteria_wrong_pi() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);
    let attacker = Address::generate(&env);

    let rules = Vec::from_array(&env, [make_rule(&env, "age", "18")]);
    let result = client.try_define_eligibility_criteria(&trial_id, &attacker, &rules);
    assert!(result.is_err());
}

#[test]
fn test_define_eligibility_criteria_trial_not_found() {
    let (env, _, pi, _, client) = create_test_env();

    let rules = Vec::from_array(&env, [make_rule(&env, "age", "18")]);
    let result = client.try_define_eligibility_criteria(&999u64, &pi, &rules);
    assert!(result.is_err());
}

#[test]
fn test_check_patient_eligibility_success() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let rule = make_rule(&env, "age", "18");
    client.define_eligibility_criteria(&trial_id, &pi, &Vec::from_array(&env, [rule.clone()]));

    let patient_data_hash = create_protocol_hash(&env);
    let claim_hash = expected_claim_hash(&env, trial_id, &patient_data_hash, &rule);
    let eligible = client.check_patient_eligibility(&trial_id, &patient_data_hash, &claim_hash);
    assert!(eligible);
}

#[test]
fn test_check_patient_eligibility_trial_not_found() {
    let (env, _, _, _, client) = create_test_env();

    let patient_data_hash = create_protocol_hash(&env);
    let rule = make_rule(&env, "age", "18");
    let claim_hash = expected_claim_hash(&env, 999u64, &patient_data_hash, &rule);
    let result = client.try_check_patient_eligibility(&999u64, &patient_data_hash, &claim_hash);
    assert!(result.is_err());
}

#[test]
fn test_set_consent_version_success() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    client.set_consent_version(&trial_id, &pi, &String::from_str(&env, "v2.0"));

    let version = client.get_consent_version(&trial_id);
    assert_eq!(version, String::from_str(&env, "v2.0"));
}

#[test]
fn test_set_consent_version_wrong_pi() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);
    let attacker = Address::generate(&env);

    let result = client.try_set_consent_version(&trial_id, &attacker, &String::from_str(&env, "v2.0"));
    assert!(result.is_err());
}

#[test]
fn test_set_consent_version_trial_not_found() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_set_consent_version(&999u64, &pi, &String::from_str(&env, "v2.0"));
    assert!(result.is_err());
}

#[test]
fn test_record_protocol_deviation_success() {
    let (env, _, pi, patient, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let enrollment_id = client.enroll_participant(
        &trial_id,
        &patient,
        &symbol_short!("armA"),
        &1100,
        &create_protocol_hash(&env),
        &String::from_str(&env, "P848"),
    );

    let deviation_id = client.record_protocol_deviation(
        &enrollment_id,
        &pi,
        &symbol_short!("visit"),
        &String::from_str(&env, "Missed scheduled visit"),
        &1200,
    );

    let deviation = client.get_protocol_deviation(&deviation_id, &pi);
    assert_eq!(deviation.enrollment_id, enrollment_id);
}

#[test]
fn test_record_protocol_deviation_wrong_pi() {
    let (env, _, pi, patient, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let enrollment_id = client.enroll_participant(
        &trial_id,
        &patient,
        &symbol_short!("armA"),
        &1100,
        &create_protocol_hash(&env),
        &String::from_str(&env, "P848"),
    );
    let attacker = Address::generate(&env);

    let result = client.try_record_protocol_deviation(
        &enrollment_id,
        &attacker,
        &symbol_short!("visit"),
        &String::from_str(&env, "Missed scheduled visit"),
        &1200,
    );
    assert!(result.is_err());
}

#[test]
fn test_record_protocol_deviation_trial_not_found() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_record_protocol_deviation(
        &999u64,
        &pi,
        &symbol_short!("visit"),
        &String::from_str(&env, "Missed scheduled visit"),
        &1200,
    );
    assert!(result.is_err());
}

#[test]
fn test_submit_safety_report_success() {
    let (env, _, pi, patient, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let enrollment_id = client.enroll_participant(
        &trial_id,
        &patient,
        &symbol_short!("armA"),
        &1100,
        &create_protocol_hash(&env),
        &String::from_str(&env, "P848"),
    );

    let report_id = client.submit_safety_report(
        &enrollment_id,
        &pi,
        &symbol_short!("serious"),
        &String::from_str(&env, "Unexpected adverse reaction"),
        &1300,
    );

    let report = client.get_safety_report(&report_id, &pi);
    assert_eq!(report.enrollment_id, enrollment_id);
}

#[test]
fn test_submit_safety_report_wrong_pi() {
    let (env, _, pi, patient, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let enrollment_id = client.enroll_participant(
        &trial_id,
        &patient,
        &symbol_short!("armA"),
        &1100,
        &create_protocol_hash(&env),
        &String::from_str(&env, "P848"),
    );
    let attacker = Address::generate(&env);

    let result = client.try_submit_safety_report(
        &enrollment_id,
        &attacker,
        &symbol_short!("serious"),
        &String::from_str(&env, "Unexpected adverse reaction"),
        &1300,
    );
    assert!(result.is_err());
}

#[test]
fn test_submit_safety_report_trial_not_found() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_submit_safety_report(
        &999u64,
        &pi,
        &symbol_short!("serious"),
        &String::from_str(&env, "Unexpected adverse reaction"),
        &1300,
    );
    assert!(result.is_err());
}

#[test]
fn test_update_protocol_success() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    let new_hash = create_protocol_hash(&env);
    client.update_protocol(
        &trial_id,
        &pi,
        &new_hash,
        &String::from_str(&env, "Protocol amendment for safety"),
    );

    let trial = client.get_trial(&trial_id);
    assert_eq!(trial.protocol_hash, new_hash);
}

#[test]
fn test_update_protocol_wrong_pi() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);
    let attacker = Address::generate(&env);

    let result = client.try_update_protocol(
        &trial_id,
        &attacker,
        &create_protocol_hash(&env),
        &String::from_str(&env, "Protocol amendment for safety"),
    );
    assert!(result.is_err());
}

#[test]
fn test_update_protocol_trial_not_found() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_update_protocol(
        &999u64,
        &pi,
        &create_protocol_hash(&env),
        &String::from_str(&env, "Protocol amendment for safety"),
    );
    assert!(result.is_err());
}

#[test]
fn test_get_amendment_history_success() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);

    client.update_protocol(
        &trial_id,
        &pi,
        &create_protocol_hash(&env),
        &String::from_str(&env, "Protocol amendment for safety"),
    );

    let history = client.get_amendment_history(&trial_id, &pi);
    assert_eq!(history.len(), 1);
}

#[test]
fn test_get_amendment_history_wrong_pi() {
    let (env, _, pi, _, client) = create_test_env();
    let trial_id = setup_regulatory_trial(&env, &client, &pi);
    let attacker = Address::generate(&env);

    let result = client.try_get_amendment_history(&trial_id, &attacker);
    assert!(result.is_err());
}

#[test]
fn test_get_amendment_history_trial_not_found() {
    let (env, _, pi, _, client) = create_test_env();

    let result = client.try_get_amendment_history(&999u64, &pi);
    assert!(result.is_err());
}
