# Incident Tracking & Resource Management Implementation

> Status: Implemented — closes #451, closes #457

## Overview
This document describes the implementation of two critical healthcare system features:
- **Structured Evidence Capture** for troubleshooting severe incidents
- **Report Job Resource Management** to prevent CPU/memory starvation

---

## A. Structured Incident Evidence Capture

### Problem Statement
Troubleshooting severe incidents in healthcare systems requires comprehensive diagnostic information. Without structured evidence capture, incidents lack traceable context for post-mortems.

### Solution Architecture

#### 1. **Incident Tracking Module** (`shared/src/incident_tracking.rs`)

**Core Components:**

- **IncidentSeverity** enum:
  - `Low` - Minor issues
  - `Medium` - Service degradation
  - `High` - Significant impact
  - `Critical` - System failure/patient safety risk

- **EvidenceType** enum:
  - `ErrorLog` - Application error logs
  - `StateSnapshot` - Contract state at failure
  - `StackTrace` - Execution trace
  - `ContextData` - Surrounding context
  - `ValidationFailure` - Input validation errors

- **Evidence** struct:
  ```rust
  pub struct Evidence {
      pub evidence_type: EvidenceType,
      pub hash: Bytes,           // Hash of evidence content
      pub recorded_at: u64,
      pub recorded_by: Address,
  }
  ```

- **Incident** struct:
  ```rust
  pub struct Incident {
      pub incident_id: u64,
      pub severity: IncidentSeverity,
      pub contract: String,      // Source contract
      pub error_code: u32,       // Standardized error ID
      pub description: String,
      pub reported_at: u64,
      pub reported_by: Address,
      pub evidence_count: u32,
      pub resolved: bool,
      pub resolution_note: Option<String>,
  }
  ```

#### 2. **Integration with Allergy Management**

**New Contract Methods:**

```rust
// Create incident with evidence capture
pub fn capture_incident(
    env: Env,
    error_code: u32,
    description: String,
    severity_level: Symbol,
    reporter: Address,
) -> Result<u64, Error>

// Attach diagnostic evidence
pub fn attach_incident_evidence(
    env: Env,
    incident_id: u64,
    evidence_type: Symbol,
    evidence_hash: Bytes,
    recorder: Address,
) -> Result<u32, Error>

// Retrieve incident details
pub fn get_incident_details(env: Env, incident_id: u64) -> Result<(u64, u32, bool), Error>

// Mark as resolved with notes
pub fn resolve_incident(
    env: Env,
    incident_id: u64,
    admin: Address,
    resolution_note: String,
) -> Result<(), Error>
```

**Event for Monitoring:**
```rust
#[contractevent]
pub struct IncidentCaptured {
    pub version: u32,
    pub incident_id: u64,
    pub severity: Symbol,
    pub contract: String,
}
```

#### 3. **How It Works**

1. **Incident Creation**:
   - Contract calls `capture_incident()` when critical error occurs
   - Incident assigned unique ID and severity level
   - Recorded with timestamp and reporter address
   - Added to open incidents tracking

2. **Evidence Attachment**:
   - Multiple evidence pieces attached to single incident
   - Max 100 evidence entries per incident (prevents bloat)
   - Each evidence hashed (privacy-preserving)
   - Evidence indexed and timestamped

3. **Troubleshooting Workflow**:
   - Admin retrieves incident ID
   - Examines incident metadata and severity
   - Retrieves attached evidence for root cause analysis
   - Marks incident resolved with notes

#### 4. **Storage Layout**
```
IncidentKey::IncidentCounter       → u64 (total incidents)
IncidentKey::Incident(id)          → Incident struct
IncidentKey::IncidentEvidence(id, idx) → Evidence struct
IncidentKey::OpenIncidents         → Vec<u64> (unresolved)
IncidentKey::ContractIncidents(contract) → Vec<u64> (by contract)
```

---

## B. Report Job Resource Management

### Problem Statement
Report generation jobs can consume excessive CPU/memory and starve other critical tasks. Without resource limits, analytics queries monopolize system resources.

### Solution Architecture

#### 1. **Resource Management Module** (`shared/src/resource_management.rs`)

**Core Components:**

- **JobPriority** enum (ordered):
  ```rust
  pub enum JobPriority {
      Low,
      Normal,
      High,
      Critical,
  }
  ```

- **ResourceQuota** struct:
  ```rust
  pub struct ResourceQuota {
      pub cpu_units: u64,       // Estimated CPU cost
      pub memory_units: u64,    // Estimated memory units
      pub timeout_seconds: u64, // Max execution time
  }
  ```

- **JobState** enum:
  - `Queued` - Waiting to run
  - `Running` - Actively executing
  - `Completed` - Successfully finished
  - `Failed` - Execution error
  - `Throttled` - Deferred due to resource limits

- **ReportJob** struct:
  ```rust
  pub struct ReportJob {
      pub job_id: u64,
      pub job_type: String,
      pub priority: JobPriority,
      pub requested_by: Address,
      pub quota: ResourceQuota,
      pub usage: Option<ResourceUsage>,
      pub state: JobState,
      pub created_at: u64,
  }
  ```

- **SystemResourceLimits** struct:
  ```rust
  pub struct SystemResourceLimits {
      pub max_concurrent_jobs: u32,
      pub total_cpu_budget: u64,    // Per ledger
      pub total_memory_budget: u64,
      pub throttle_threshold: u64,  // % at which to throttle
  }
  ```

#### 2. **Integration with Healthcare Analytics**

**New Contract Methods:**

```rust
// Request report job creation (with resource estimation)
pub fn request_report(
    env: Env,
    requester: Address,
    report_type: String,
    priority: JobPriority,
    estimated_cpu: u64,
    estimated_memory: u64,
) -> Result<u64, Error>

// Execute next available job respecting resource limits
pub fn execute_next_report(env: Env, admin: Address) -> Result<Option<u64>, Error>

// Mark job as completed with actual resource usage (executor/oracle or admin only)
pub fn complete_report(
    env: Env,
    executor: Address,
    job_id: u64,
    cpu_used: u64,
    memory_used: u64,
    wall_time_ms: u64,
) -> Result<(), Error>

// Get current resource limits
pub fn get_resource_limits(env: Env) -> (u64, u64, u32, u64)

// Admin: Update resource limits
pub fn set_resource_limits(
    env: Env,
    admin: Address,
    cpu_budget: u64,
    memory_budget: u64,
    max_concurrent: u32,
    throttle_percent: u64,
) -> Result<(), Error>
```

**Error Codes:**
- `JobThrottled` - System at resource threshold, job deferred
- `InsufficientResources` - Not enough budget for requested quota
- `JobNotFound` - Invalid job ID

#### 3. **How It Works**

1. **Job Request**:
   - Client requests report with estimated CPU/memory
   - System checks: throttle threshold, available budget, concurrency limit
   - Job queued or rejected with throttle error
   - High-priority jobs get preferential scheduling

2. **Resource Tracking**:
   ```
   Total Budget (per ledger):
   ├─ CPU Budget: 10,000,000 units
   └─ Memory Budget: 1,000,000 units
   
   Throttle Trigger: 80% consumption
   Max Concurrent: 5 jobs
   ```

3. **Job Execution**:
   - Next-job-for-execution selects highest priority queued job
   - Job moves from Queued → Running
   - Execution tracks actual resource consumption
   - Upon completion, records usage and frees resources

4. **Throttling Logic**:
   - If `(cpu_used * 100) / budget > threshold` → new jobs rejected with `JobThrottled`
   - Low/Normal priority jobs defer until budget available
   - High/Critical jobs attempt execution anyway (may overshoot)
   - Each ledger resets budget counters

#### 4. **Storage Layout**
```
ResourceKey::JobCounter          → u64
ResourceKey::ReportJob(id)       → ReportJob struct
ResourceKey::QueuedJobs          → Vec<u64> (job queue)
ResourceKey::RunningJobs         → Vec<u64> (executing)
ResourceKey::SystemLimits        → SystemResourceLimits
ResourceKey::TotalCpuUsed        → u64 (current period)
ResourceKey::TotalMemoryUsed     → u64 (current period)
```

#### 5. **Default Configuration**
```rust
DEFAULT_CPU_QUOTA:         1,000,000 units
DEFAULT_MEMORY_QUOTA:      100,000 units
DEFAULT_TIMEOUT:           300 seconds (5 min)
DEFAULT_MAX_CONCURRENT:    5 jobs
DEFAULT_CPU_BUDGET:        10,000,000 units
DEFAULT_MEMORY_BUDGET:     1,000,000 units
DEFAULT_THROTTLE_THRESHOLD: 80%
```

---

## Security Considerations

### Incident Tracking
- **Privacy**: Evidence stored as hashes, not raw data
- **Access Control**: `require_auth()` on capture/attach operations
- **Max Evidence**: 100 per incident prevents storage attacks
- **Immutability**: Incidents archived, cannot be deleted

### Resource Management
- **Priority Queuing**: Prevents low-priority job starvation indefinitely
- **Budget Enforcement**: Hard limits on CPU/memory per period
- **Throttle Mechanism**: Graceful degradation vs. rejection
- **Admin Controls**: Only admins can modify resource limits
- **Metering**: Actual usage tracked for cost accountability

---

## Integration Example

```rust
// Example: Handling an error with incident capture
pub fn record_allergy(...) -> Result<u64, Error> {
    if !Self::is_registered_provider(&env, &provider_id) {
        // Capture incident for troubleshooting
        let incident_id = AllergyManagement::capture_incident(
            env,
            401, // error code
            String::from_str(&env, "Unauthorized provider access attempt"),
            symbol_short!("high"),
            provider_id,
        )?;
        
        // Attach evidence
        let evidence_hash = env.crypto().sha256(&provider_id.to_xdr(&env)).into();
        AllergyManagement::attach_incident_evidence(
            env,
            incident_id,
            symbol_short!("context"),
            evidence_hash,
            provider_id,
        )?;
        
        return Err(Error::Unauthorized);
    }
    // ... continue with normal flow
}

// Example: Request resource-managed report
pub fn generate_adverse_events_report(...) -> Result<u64, Error> {
    let job_id = HealthcareAnalytics::request_report(
        env,
        requester,
        String::from_str(&env, "adverse_event_report"),
        JobPriority::High,
        5_000_000, // estimated CPU units
        500_000,   // estimated memory units
    )?;
    
    // Later: execute next job
    if let Some(executing_job) = HealthcareAnalytics::execute_next_report(env, admin)? {
        // ... perform report generation ...
        
        // Record actual resource usage (executor/oracle reports it, not the requester)
        HealthcareAnalytics::complete_report(env, executor, executing_job, 3_200_000, 450_000, 1_500)?;
    }
    
    Ok(job_id)
}
```

---

## Monitoring & Alerts

### Events Published
- `IncidentCaptured` - New incident recorded
- `report_job` - New job requested
- `exec_job` - Job started
- `job_done` - Job completed with usage stats

### Metrics to Track
1. **Incident Metrics**:
   - Open incidents by severity
   - Evidence count per incident
   - Resolution time
   - Incidents by contract

2. **Resource Metrics**:
   - Queue depth
   - Job throughput
   - CPU/memory usage vs. budget
   - Throttle rate
   - Priority distribution

---

## Future Enhancements

1. **Incident Analytics**: Dashboard showing incident trends
2. **Auto-Escalation**: Escalate unresolved critical incidents
3. **Resource Predictions**: ML-based job resource estimation
4. **Dynamic Throttling**: Adjust thresholds based on system state
5. **Cross-Contract Incidents**: Correlate incidents across related contracts
