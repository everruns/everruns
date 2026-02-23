//! Fake AWS Tools Capability - mock AWS infrastructure management
//!
//! All state is persisted in the session file system under `/aws/`.
//! Every tool call simulates realistic AWS API latency (configurable via
//! `FAKE_AWS_LATENCY_MS` env var; default 0 = no delay, set e.g. 200 for
//! benchmarking long-running agents).
//!
//! Tools provided:
//! - `aws_list_ec2_instances`: List EC2 instances
//! - `aws_create_ec2_instance`: Launch a new EC2 instance
//! - `aws_stop_ec2_instance`: Stop an EC2 instance
//! - `aws_list_rds_databases`: List RDS database instances
//! - `aws_create_rds_database`: Create a new RDS database
//! - `aws_list_s3_buckets`: List S3 buckets
//! - `aws_create_s3_bucket`: Create a new S3 bucket
//! - `aws_list_iam_users`: List IAM users
//! - `aws_create_iam_user`: Create a new IAM user
//! - `aws_list_security_groups`: List security groups
//! - `aws_get_cloudwatch_metrics`: Get CloudWatch metrics

use super::{Capability, CapabilityStatus};
use crate::SessionId;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileStore, ToolContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::time::Duration;

// ============================================================================
// Latency simulation
// ============================================================================

/// Base latency loaded once from `FAKE_AWS_LATENCY_MS` (default: 0).
fn base_latency_ms() -> u64 {
    static BASE: OnceLock<u64> = OnceLock::new();
    *BASE.get_or_init(|| {
        std::env::var("FAKE_AWS_LATENCY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    })
}

/// Operation-type multipliers applied to the base latency.
#[derive(Clone, Copy)]
enum OpKind {
    /// List / describe operations (~1x base)
    List,
    /// Create / launch operations (~3x base)
    Create,
    /// Modify / stop operations (~2x base)
    Modify,
    /// Metrics / query operations (~1.5x base)
    Query,
}

impl OpKind {
    fn multiplier(self) -> f64 {
        match self {
            OpKind::List => 1.0,
            OpKind::Create => 3.0,
            OpKind::Modify => 2.0,
            OpKind::Query => 1.5,
        }
    }
}

/// Deterministic jitter derived from current timestamp sub-micros (no rand dep).
fn jitter_factor() -> f64 {
    let nanos = chrono::Utc::now().timestamp_subsec_nanos();
    // Produces a value in [0.8, 1.2)
    0.8 + (nanos % 400) as f64 / 1000.0
}

/// Sleep to simulate AWS API latency. No-op when base is 0.
async fn simulate_latency(op: OpKind) {
    let base = base_latency_ms();
    if base == 0 {
        return;
    }
    let ms = (base as f64 * op.multiplier() * jitter_factor()) as u64;
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

// ============================================================================
// Persistence helpers
// ============================================================================

/// Read a JSON collection from the session file store, returning seed data on
/// first access and persisting it.
async fn read_or_seed<T: Serialize + for<'de> Deserialize<'de>>(
    store: &dyn SessionFileStore,
    session_id: SessionId,
    path: &str,
    seed: impl FnOnce() -> Vec<T>,
) -> Vec<T> {
    if let Ok(Some(file)) = store.read_file(session_id, path).await
        && let Ok(items) = serde_json::from_str::<Vec<T>>(file.content.as_deref().unwrap_or(""))
    {
        return items;
    }
    // First access — persist seed data
    let items = seed();
    if let Ok(json) = serde_json::to_string_pretty(&items) {
        let _ = store.write_file(session_id, path, &json, "text").await;
    }
    items
}

/// Persist a collection back to the session file store.
async fn persist<T: Serialize>(
    store: &dyn SessionFileStore,
    session_id: SessionId,
    path: &str,
    items: &[T],
) -> Result<(), anyhow::Error> {
    let json = serde_json::to_string_pretty(items)?;
    store.write_file(session_id, path, &json, "text").await?;
    Ok(())
}

/// Extract the file store from context or return a tool error.
macro_rules! require_file_store {
    ($ctx:expr) => {
        match &$ctx.file_store {
            Some(s) => s.as_ref(),
            None => return ToolExecutionResult::tool_error("File system not available"),
        }
    };
}

// ============================================================================
// Data model structs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ec2Instance {
    instance_id: String,
    instance_type: String,
    state: String,
    availability_zone: String,
    private_ip: String,
    public_ip: Option<String>,
    launch_time: String,
    tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tag {
    key: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RdsDatabase {
    db_instance_id: String,
    engine: String,
    engine_version: String,
    instance_class: String,
    status: String,
    endpoint: String,
    port: i32,
    storage_gb: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct S3Bucket {
    name: String,
    region: String,
    creation_date: String,
    versioning_enabled: bool,
    encryption_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IamUser {
    username: String,
    user_id: String,
    arn: String,
    created_at: String,
    permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecurityGroup {
    group_id: String,
    group_name: String,
    description: String,
    vpc_id: String,
    inbound_rules: Vec<SecurityRule>,
    outbound_rules: Vec<SecurityRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecurityRule {
    protocol: String,
    port_range: String,
    source: String,
}

// ============================================================================
// Seed data factories
// ============================================================================

const EC2_PATH: &str = "/aws/ec2_instances.json";
const RDS_PATH: &str = "/aws/rds_databases.json";
const S3_PATH: &str = "/aws/s3_buckets.json";
const IAM_PATH: &str = "/aws/iam_users.json";
const SG_PATH: &str = "/aws/security_groups.json";

fn seed_ec2() -> Vec<Ec2Instance> {
    vec![
        // --- Healthy production instances ---
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60001".into(),
            instance_type: "t3.medium".into(),
            state: "running".into(),
            availability_zone: "us-east-1a".into(),
            private_ip: "10.0.1.10".into(),
            public_ip: Some("54.200.10.1".into()),
            launch_time: "2025-06-15T08:00:00Z".into(),
            tags: vec![
                Tag {
                    key: "Name".into(),
                    value: "web-server-01".into(),
                },
                Tag {
                    key: "Environment".into(),
                    value: "production".into(),
                },
                Tag {
                    key: "Team".into(),
                    value: "platform".into(),
                },
            ],
        },
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60002".into(),
            instance_type: "t3.medium".into(),
            state: "running".into(),
            availability_zone: "us-east-1b".into(),
            private_ip: "10.0.2.10".into(),
            public_ip: Some("54.200.10.2".into()),
            launch_time: "2025-06-15T08:05:00Z".into(),
            tags: vec![
                Tag {
                    key: "Name".into(),
                    value: "web-server-02".into(),
                },
                Tag {
                    key: "Environment".into(),
                    value: "production".into(),
                },
                Tag {
                    key: "Team".into(),
                    value: "platform".into(),
                },
            ],
        },
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60003".into(),
            instance_type: "t3.large".into(),
            state: "running".into(),
            availability_zone: "us-east-1a".into(),
            private_ip: "10.0.1.20".into(),
            public_ip: Some("54.200.10.3".into()),
            launch_time: "2025-07-01T10:00:00Z".into(),
            tags: vec![
                Tag {
                    key: "Name".into(),
                    value: "api-server-01".into(),
                },
                Tag {
                    key: "Environment".into(),
                    value: "production".into(),
                },
                Tag {
                    key: "Team".into(),
                    value: "backend".into(),
                },
            ],
        },
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60004".into(),
            instance_type: "t3.large".into(),
            state: "running".into(),
            availability_zone: "us-east-1b".into(),
            private_ip: "10.0.2.20".into(),
            public_ip: Some("54.200.10.4".into()),
            launch_time: "2025-07-01T10:05:00Z".into(),
            tags: vec![
                Tag {
                    key: "Name".into(),
                    value: "api-server-02".into(),
                },
                Tag {
                    key: "Environment".into(),
                    value: "production".into(),
                },
                Tag {
                    key: "Team".into(),
                    value: "backend".into(),
                },
            ],
        },
        // --- ISSUE: Idle expensive instance (m5.xlarge barely doing anything) ---
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60005".into(),
            instance_type: "m5.xlarge".into(),
            state: "running".into(),
            availability_zone: "us-east-1a".into(),
            private_ip: "10.0.1.30".into(),
            public_ip: Some("54.200.10.5".into()),
            launch_time: "2025-03-10T09:00:00Z".into(),
            tags: vec![
                Tag {
                    key: "Name".into(),
                    value: "batch-processor-01".into(),
                },
                Tag {
                    key: "Environment".into(),
                    value: "production".into(),
                },
            ],
        },
        // --- ISSUE: Old-gen instance type (m4), missing Environment tag ---
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60006".into(),
            instance_type: "m4.large".into(),
            state: "running".into(),
            availability_zone: "us-east-1a".into(),
            private_ip: "10.0.1.40".into(),
            public_ip: Some("54.200.10.6".into()),
            launch_time: "2024-11-01T12:00:00Z".into(),
            tags: vec![Tag {
                key: "Name".into(),
                value: "legacy-app-01".into(),
            }],
        },
        // --- ISSUE: Dev instance running for months, wasting money ---
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60007".into(),
            instance_type: "t3.small".into(),
            state: "running".into(),
            availability_zone: "us-east-1a".into(),
            private_ip: "10.0.1.50".into(),
            public_ip: Some("54.200.10.7".into()),
            launch_time: "2025-01-15T14:00:00Z".into(),
            tags: vec![
                Tag {
                    key: "Name".into(),
                    value: "test-server-01".into(),
                },
                Tag {
                    key: "Environment".into(),
                    value: "development".into(),
                },
                Tag {
                    key: "Owner".into(),
                    value: "intern-temp".into(),
                },
            ],
        },
        // --- ISSUE: Stopped expensive instance, NO tags at all ---
        Ec2Instance {
            instance_id: "i-0a1b2c3d4e5f60008".into(),
            instance_type: "c5.2xlarge".into(),
            state: "stopped".into(),
            availability_zone: "us-east-1b".into(),
            private_ip: "10.0.2.60".into(),
            public_ip: None,
            launch_time: "2025-02-01T08:00:00Z".into(),
            tags: vec![],
        },
    ]
}

fn seed_rds() -> Vec<RdsDatabase> {
    vec![
        // Healthy production database
        RdsDatabase {
            db_instance_id: "prod-postgres-01".into(),
            engine: "postgres".into(),
            engine_version: "15.4".into(),
            instance_class: "db.t3.medium".into(),
            status: "available".into(),
            endpoint: "prod-postgres-01.abc123.us-east-1.rds.amazonaws.com".into(),
            port: 5432,
            storage_gb: 100,
        },
        // ISSUE: Oversized instance for what is actually a staging workload
        RdsDatabase {
            db_instance_id: "staging-mysql-01".into(),
            engine: "mysql".into(),
            engine_version: "8.0".into(),
            instance_class: "db.r5.large".into(),
            status: "available".into(),
            endpoint: "staging-mysql-01.abc123.us-east-1.rds.amazonaws.com".into(),
            port: 3306,
            storage_gb: 500,
        },
        // ISSUE: End-of-life Postgres 11 (EOL Nov 2023)
        RdsDatabase {
            db_instance_id: "legacy-pg-11".into(),
            engine: "postgres".into(),
            engine_version: "11.22".into(),
            instance_class: "db.t3.small".into(),
            status: "available".into(),
            endpoint: "legacy-pg-11.abc123.us-east-1.rds.amazonaws.com".into(),
            port: 5432,
            storage_gb: 20,
        },
    ]
}

fn seed_s3() -> Vec<S3Bucket> {
    vec![
        // Healthy: properly configured backup bucket
        S3Bucket {
            name: "company-data-backup".into(),
            region: "us-east-1".into(),
            creation_date: "2024-06-01T00:00:00Z".into(),
            versioning_enabled: true,
            encryption_enabled: true,
        },
        // ISSUE: Production assets without versioning
        S3Bucket {
            name: "static-assets-prod".into(),
            region: "us-east-1".into(),
            creation_date: "2024-08-15T00:00:00Z".into(),
            versioning_enabled: false,
            encryption_enabled: true,
        },
        // ISSUE: No encryption AND no versioning — dump bucket
        S3Bucket {
            name: "temp-data-dump".into(),
            region: "us-east-1".into(),
            creation_date: "2025-04-01T00:00:00Z".into(),
            versioning_enabled: false,
            encryption_enabled: false,
        },
        // CRITICAL: PII bucket without encryption!
        S3Bucket {
            name: "customer-pii-records".into(),
            region: "us-east-1".into(),
            creation_date: "2024-09-10T00:00:00Z".into(),
            versioning_enabled: true,
            encryption_enabled: false,
        },
        // ISSUE: Stale dev bucket, no encryption, no versioning
        S3Bucket {
            name: "dev-test-artifacts".into(),
            region: "us-west-2".into(),
            creation_date: "2025-01-20T00:00:00Z".into(),
            versioning_enabled: false,
            encryption_enabled: false,
        },
        // Healthy: well-configured archive
        S3Bucket {
            name: "log-archive-2024".into(),
            region: "us-east-1".into(),
            creation_date: "2024-01-01T00:00:00Z".into(),
            versioning_enabled: true,
            encryption_enabled: true,
        },
    ]
}

fn seed_iam() -> Vec<IamUser> {
    vec![
        // ISSUE: Human user with full admin — should use roles instead
        IamUser {
            username: "admin-user".into(),
            user_id: "AIDAI23XYZABC123DEF".into(),
            arn: "arn:aws:iam::123456789012:user/admin-user".into(),
            created_at: "2024-01-15T10:00:00Z".into(),
            permissions: vec!["AdministratorAccess".into()],
        },
        // ISSUE: Overly broad PowerUserAccess
        IamUser {
            username: "developer-user".into(),
            user_id: "AIDAI23XYZABC456GHI".into(),
            arn: "arn:aws:iam::123456789012:user/developer-user".into(),
            created_at: "2024-02-20T14:30:00Z".into(),
            permissions: vec!["PowerUserAccess".into()],
        },
        // CRITICAL: CI bot with AdministratorAccess — massive risk
        IamUser {
            username: "ci-deploy-bot".into(),
            user_id: "AIDAI23XYZABC789JKL".into(),
            arn: "arn:aws:iam::123456789012:user/ci-deploy-bot".into(),
            created_at: "2024-06-01T09:00:00Z".into(),
            permissions: vec!["AdministratorAccess".into(), "IAMFullAccess".into()],
        },
        // ISSUE: Temp intern account with excessive permissions
        IamUser {
            username: "intern-temp".into(),
            user_id: "AIDAI23XYZABCMNOPQR".into(),
            arn: "arn:aws:iam::123456789012:user/intern-temp".into(),
            created_at: "2025-06-01T10:00:00Z".into(),
            permissions: vec![
                "AmazonS3FullAccess".into(),
                "AmazonEC2FullAccess".into(),
                "AmazonRDSFullAccess".into(),
            ],
        },
        // Healthy: monitoring service with read-only access
        IamUser {
            username: "monitoring-svc".into(),
            user_id: "AIDAI23XYZABCSTUVWX".into(),
            arn: "arn:aws:iam::123456789012:user/monitoring-svc".into(),
            created_at: "2024-03-01T08:00:00Z".into(),
            permissions: vec!["CloudWatchReadOnlyAccess".into()],
        },
        // Mostly OK: analyst with appropriately scoped access
        IamUser {
            username: "data-analyst".into(),
            user_id: "AIDAI23XYZABCYZABCD".into(),
            arn: "arn:aws:iam::123456789012:user/data-analyst".into(),
            created_at: "2024-08-15T11:00:00Z".into(),
            permissions: vec![
                "AmazonS3ReadOnlyAccess".into(),
                "AmazonAthenaFullAccess".into(),
            ],
        },
    ]
}

fn seed_security_groups() -> Vec<SecurityGroup> {
    vec![
        // Mostly OK: standard web-facing security group
        SecurityGroup {
            group_id: "sg-0a1b2c3d4e5f60001".into(),
            group_name: "web-server-sg".into(),
            description: "Security group for web servers".into(),
            vpc_id: "vpc-abc123".into(),
            inbound_rules: vec![
                SecurityRule {
                    protocol: "tcp".into(),
                    port_range: "80".into(),
                    source: "0.0.0.0/0".into(),
                },
                SecurityRule {
                    protocol: "tcp".into(),
                    port_range: "443".into(),
                    source: "0.0.0.0/0".into(),
                },
            ],
            outbound_rules: vec![SecurityRule {
                protocol: "-1".into(),
                port_range: "all".into(),
                source: "0.0.0.0/0".into(),
            }],
        },
        // CRITICAL: Database ports open to the entire internet!
        SecurityGroup {
            group_id: "sg-0a1b2c3d4e5f60002".into(),
            group_name: "database-sg".into(),
            description: "Security group for databases".into(),
            vpc_id: "vpc-abc123".into(),
            inbound_rules: vec![
                SecurityRule {
                    protocol: "tcp".into(),
                    port_range: "5432".into(),
                    source: "0.0.0.0/0".into(),
                },
                SecurityRule {
                    protocol: "tcp".into(),
                    port_range: "3306".into(),
                    source: "0.0.0.0/0".into(),
                },
            ],
            outbound_rules: vec![SecurityRule {
                protocol: "-1".into(),
                port_range: "all".into(),
                source: "0.0.0.0/0".into(),
            }],
        },
        // CRITICAL: SSH and RDP open to the world
        SecurityGroup {
            group_id: "sg-0a1b2c3d4e5f60003".into(),
            group_name: "admin-access-sg".into(),
            description: "Security group for admin access".into(),
            vpc_id: "vpc-abc123".into(),
            inbound_rules: vec![
                SecurityRule {
                    protocol: "tcp".into(),
                    port_range: "22".into(),
                    source: "0.0.0.0/0".into(),
                },
                SecurityRule {
                    protocol: "tcp".into(),
                    port_range: "3389".into(),
                    source: "0.0.0.0/0".into(),
                },
            ],
            outbound_rules: vec![SecurityRule {
                protocol: "-1".into(),
                port_range: "all".into(),
                source: "0.0.0.0/0".into(),
            }],
        },
    ]
}

// ============================================================================
// CloudWatch metric profiles for seed resources
// ============================================================================

/// Per-resource metric base values. Overrides the generic default so seed
/// instances tell a realistic story the auditor can discover.
fn metric_profile_base(resource_id: &str, metric: &str) -> Option<f64> {
    match (resource_id, metric) {
        // web-server-01: healthy, moderate-high load
        ("i-0a1b2c3d4e5f60001", "CPUUtilization") => Some(62.0),
        ("i-0a1b2c3d4e5f60001", "MemoryUtilization") => Some(71.0),
        ("i-0a1b2c3d4e5f60001", "NetworkIn") => Some(85_000.0),

        // web-server-02: healthy, moderate load
        ("i-0a1b2c3d4e5f60002", "CPUUtilization") => Some(48.0),
        ("i-0a1b2c3d4e5f60002", "MemoryUtilization") => Some(55.0),
        ("i-0a1b2c3d4e5f60002", "NetworkIn") => Some(72_000.0),

        // api-server-01: healthy, moderate load
        ("i-0a1b2c3d4e5f60003", "CPUUtilization") => Some(45.0),
        ("i-0a1b2c3d4e5f60003", "MemoryUtilization") => Some(60.0),
        ("i-0a1b2c3d4e5f60003", "NetworkIn") => Some(95_000.0),

        // api-server-02: healthy, moderate load
        ("i-0a1b2c3d4e5f60004", "CPUUtilization") => Some(42.0),
        ("i-0a1b2c3d4e5f60004", "MemoryUtilization") => Some(58.0),
        ("i-0a1b2c3d4e5f60004", "NetworkIn") => Some(88_000.0),

        // batch-processor-01: IDLE — nearly zero utilization on expensive m5.xlarge
        ("i-0a1b2c3d4e5f60005", "CPUUtilization") => Some(2.1),
        ("i-0a1b2c3d4e5f60005", "MemoryUtilization") => Some(5.3),
        ("i-0a1b2c3d4e5f60005", "NetworkIn") => Some(320.0),

        // legacy-app-01: very low utilization on old-gen m4
        ("i-0a1b2c3d4e5f60006", "CPUUtilization") => Some(4.8),
        ("i-0a1b2c3d4e5f60006", "MemoryUtilization") => Some(12.0),
        ("i-0a1b2c3d4e5f60006", "NetworkIn") => Some(1_200.0),

        // test-server-01: minimal dev traffic
        ("i-0a1b2c3d4e5f60007", "CPUUtilization") => Some(3.2),
        ("i-0a1b2c3d4e5f60007", "MemoryUtilization") => Some(9.0),
        ("i-0a1b2c3d4e5f60007", "NetworkIn") => Some(800.0),

        // prod-postgres-01: healthy RDS
        ("prod-postgres-01", "CPUUtilization") => Some(38.0),
        ("prod-postgres-01", "MemoryUtilization") => Some(65.0),

        // staging-mysql-01: underutilized oversized RDS
        ("staging-mysql-01", "CPUUtilization") => Some(8.5),
        ("staging-mysql-01", "MemoryUtilization") => Some(15.0),

        // legacy-pg-11: EOL database, low load
        ("legacy-pg-11", "CPUUtilization") => Some(6.0),
        ("legacy-pg-11", "MemoryUtilization") => Some(22.0),

        _ => None,
    }
}

// ============================================================================
// Capability
// ============================================================================

pub struct FakeAwsCapability;

impl Capability for FakeAwsCapability {
    fn id(&self) -> &str {
        "fake_aws"
    }

    fn name(&self) -> &str {
        "Fake AWS Tools"
    }

    fn description(&self) -> &str {
        "Mock AWS infrastructure tools (EC2, RDS, S3, IAM, CloudWatch). \
         All state persisted in session filesystem. Latency emulation via FAKE_AWS_LATENCY_MS."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud")
    }

    fn category(&self) -> Option<&str> {
        Some("Demo Tools")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to AWS infrastructure management tools. All AWS data is stored in /aws/ directory.

Available tools:
- `aws_list_ec2_instances`: List all EC2 instances with their status
- `aws_create_ec2_instance`: Launch a new EC2 instance
- `aws_stop_ec2_instance`: Stop a running EC2 instance
- `aws_list_rds_databases`: List RDS database instances
- `aws_create_rds_database`: Create a new RDS database
- `aws_list_s3_buckets`: List all S3 buckets
- `aws_create_s3_bucket`: Create a new S3 bucket
- `aws_list_iam_users`: List IAM users and their permissions
- `aws_create_iam_user`: Create a new IAM user
- `aws_list_security_groups`: List security groups and rules
- `aws_get_cloudwatch_metrics`: Get CloudWatch metrics for resources

Data structure:
- /aws/ec2_instances.json - EC2 instance records
- /aws/rds_databases.json - RDS database records
- /aws/s3_buckets.json - S3 bucket records
- /aws/iam_users.json - IAM user records
- /aws/security_groups.json - Security group records

API calls have realistic latency. All mutations are persisted."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(AwsListEc2InstancesTool),
            Box::new(AwsCreateEc2InstanceTool),
            Box::new(AwsStopEc2InstanceTool),
            Box::new(AwsListRdsDatabasesTool),
            Box::new(AwsCreateRdsDatabaseTool),
            Box::new(AwsListS3BucketsTool),
            Box::new(AwsCreateS3BucketTool),
            Box::new(AwsListIamUsersTool),
            Box::new(AwsCreateIamUserTool),
            Box::new(AwsListSecurityGroupsTool),
            Box::new(AwsGetCloudWatchMetricsTool),
        ]
    }
}

// ============================================================================
// Tool: aws_list_ec2_instances
// ============================================================================

pub struct AwsListEc2InstancesTool;

#[async_trait]
impl Tool for AwsListEc2InstancesTool {
    fn name(&self) -> &str {
        "aws_list_ec2_instances"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List EC2 Instances")
    }

    fn description(&self) -> &str {
        "List all EC2 instances with their current status, IPs, and configuration."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "state": {
                    "type": "string",
                    "enum": ["running", "stopped", "terminated"],
                    "description": "Optional: Filter by instance state"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("aws_list_ec2_instances requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::List).await;

        let state_filter = arguments.get("state").and_then(|v| v.as_str());
        let instances = read_or_seed(store, context.session_id, EC2_PATH, seed_ec2).await;

        let filtered: Vec<_> = if let Some(state) = state_filter {
            instances.into_iter().filter(|i| i.state == state).collect()
        } else {
            instances
        };

        ToolExecutionResult::success(json!({
            "instances": filtered,
            "total_count": filtered.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_create_ec2_instance
// ============================================================================

pub struct AwsCreateEc2InstanceTool;

#[async_trait]
impl Tool for AwsCreateEc2InstanceTool {
    fn name(&self) -> &str {
        "aws_create_ec2_instance"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Create EC2 Instance")
    }

    fn description(&self) -> &str {
        "Launch a new EC2 instance with specified configuration."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "instance_type": {
                    "type": "string",
                    "description": "Instance type (e.g., 't3.micro', 't3.medium')"
                },
                "name": {
                    "type": "string",
                    "description": "Instance name tag"
                },
                "availability_zone": {
                    "type": "string",
                    "description": "Availability zone (default: us-east-1a)"
                }
            },
            "required": ["instance_type", "name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("aws_create_ec2_instance requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::Create).await;

        let instance_type = match arguments.get("instance_type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: instance_type",
                );
            }
        };
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolExecutionResult::tool_error("Missing required parameter: name"),
        };
        let az = arguments
            .get("availability_zone")
            .and_then(|v| v.as_str())
            .unwrap_or("us-east-1a");

        let mut instances = read_or_seed(store, context.session_id, EC2_PATH, seed_ec2).await;

        let instance_id = format!(
            "i-{:016x}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64
        );
        let private_ip = format!("10.0.{}.{}", (instances.len() % 254) + 1, 100);
        let public_ip = format!(
            "54.{}.{}.{}",
            100 + instances.len() % 100,
            (instances.len() % 254) + 1,
            100
        );

        let instance = Ec2Instance {
            instance_id: instance_id.clone(),
            instance_type: instance_type.to_string(),
            state: "running".into(),
            availability_zone: az.to_string(),
            private_ip: private_ip.clone(),
            public_ip: Some(public_ip.clone()),
            launch_time: chrono::Utc::now().to_rfc3339(),
            tags: vec![Tag {
                key: "Name".into(),
                value: name.to_string(),
            }],
        };
        instances.push(instance);

        match persist(store, context.session_id, EC2_PATH, &instances).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "instance_id": instance_id,
                "state": "running",
                "private_ip": private_ip,
                "public_ip": public_ip,
                "message": "EC2 instance launched successfully"
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to persist: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_stop_ec2_instance
// ============================================================================

pub struct AwsStopEc2InstanceTool;

#[async_trait]
impl Tool for AwsStopEc2InstanceTool {
    fn name(&self) -> &str {
        "aws_stop_ec2_instance"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Stop EC2 Instance")
    }

    fn description(&self) -> &str {
        "Stop a running EC2 instance."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "instance_id": {
                    "type": "string",
                    "description": "Instance ID to stop"
                }
            },
            "required": ["instance_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("aws_stop_ec2_instance requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::Modify).await;

        let instance_id = match arguments.get("instance_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: instance_id");
            }
        };

        let mut instances = read_or_seed(store, context.session_id, EC2_PATH, seed_ec2).await;

        let instance = match instances.iter_mut().find(|i| i.instance_id == instance_id) {
            Some(i) => i,
            None => {
                return ToolExecutionResult::tool_error(format!(
                    "Instance not found: {}",
                    instance_id
                ));
            }
        };

        let old_state = instance.state.clone();
        instance.state = "stopped".into();

        match persist(store, context.session_id, EC2_PATH, &instances).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "instance_id": instance_id,
                "old_state": old_state,
                "new_state": "stopped"
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to persist: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_list_rds_databases
// ============================================================================

pub struct AwsListRdsDatabasesTool;

#[async_trait]
impl Tool for AwsListRdsDatabasesTool {
    fn name(&self) -> &str {
        "aws_list_rds_databases"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List RDS Databases")
    }

    fn description(&self) -> &str {
        "List all RDS database instances."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::List).await;

        let databases = read_or_seed(store, context.session_id, RDS_PATH, seed_rds).await;

        ToolExecutionResult::success(
            json!({"databases": databases, "total_count": databases.len()}),
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_create_rds_database
// ============================================================================

pub struct AwsCreateRdsDatabaseTool;

#[async_trait]
impl Tool for AwsCreateRdsDatabaseTool {
    fn name(&self) -> &str {
        "aws_create_rds_database"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Create RDS Database")
    }

    fn description(&self) -> &str {
        "Create a new RDS database instance."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "db_instance_id": {"type": "string"},
                "engine": {"type": "string", "enum": ["postgres", "mysql", "mariadb"]},
                "instance_class": {"type": "string"},
                "storage_gb": {"type": "integer"}
            },
            "required": ["db_instance_id", "engine", "instance_class"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::Create).await;

        let db_id = match arguments.get("db_instance_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: db_instance_id",
                );
            }
        };
        let engine = match arguments.get("engine").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return ToolExecutionResult::tool_error("Missing required parameter: engine"),
        };
        let instance_class = match arguments.get("instance_class").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: instance_class",
                );
            }
        };
        let storage_gb = arguments
            .get("storage_gb")
            .and_then(|v| v.as_i64())
            .unwrap_or(20) as i32;

        let engine_version = match engine {
            "postgres" => "15.4",
            "mysql" => "8.0",
            "mariadb" => "10.11",
            _ => "unknown",
        };
        let port = match engine {
            "postgres" => 5432,
            "mysql" | "mariadb" => 3306,
            _ => 5432,
        };

        let mut databases = read_or_seed(store, context.session_id, RDS_PATH, seed_rds).await;

        let db = RdsDatabase {
            db_instance_id: db_id.to_string(),
            engine: engine.to_string(),
            engine_version: engine_version.to_string(),
            instance_class: instance_class.to_string(),
            status: "creating".into(),
            endpoint: format!("{}.abc123.us-east-1.rds.amazonaws.com", db_id),
            port,
            storage_gb,
        };
        databases.push(db);

        match persist(store, context.session_id, RDS_PATH, &databases).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "db_instance_id": db_id,
                "status": "creating",
                "endpoint": format!("{}.abc123.us-east-1.rds.amazonaws.com", db_id),
                "message": "RDS database creation initiated"
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to persist: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_list_s3_buckets
// ============================================================================

pub struct AwsListS3BucketsTool;

#[async_trait]
impl Tool for AwsListS3BucketsTool {
    fn name(&self) -> &str {
        "aws_list_s3_buckets"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List S3 Buckets")
    }

    fn description(&self) -> &str {
        "List all S3 buckets in the account."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::List).await;

        let buckets = read_or_seed(store, context.session_id, S3_PATH, seed_s3).await;

        ToolExecutionResult::success(json!({"buckets": buckets, "total_count": buckets.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_create_s3_bucket
// ============================================================================

pub struct AwsCreateS3BucketTool;

#[async_trait]
impl Tool for AwsCreateS3BucketTool {
    fn name(&self) -> &str {
        "aws_create_s3_bucket"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Create S3 Bucket")
    }

    fn description(&self) -> &str {
        "Create a new S3 bucket."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "bucket_name": {"type": "string"},
                "region": {"type": "string"},
                "versioning_enabled": {"type": "boolean"},
                "encryption_enabled": {"type": "boolean"}
            },
            "required": ["bucket_name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::Create).await;

        let bucket_name = match arguments.get("bucket_name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: bucket_name");
            }
        };
        let region = arguments
            .get("region")
            .and_then(|v| v.as_str())
            .unwrap_or("us-east-1");
        let versioning = arguments
            .get("versioning_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let encryption = arguments
            .get("encryption_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut buckets = read_or_seed(store, context.session_id, S3_PATH, seed_s3).await;

        let bucket = S3Bucket {
            name: bucket_name.to_string(),
            region: region.to_string(),
            creation_date: chrono::Utc::now().to_rfc3339(),
            versioning_enabled: versioning,
            encryption_enabled: encryption,
        };
        buckets.push(bucket);

        match persist(store, context.session_id, S3_PATH, &buckets).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "bucket_name": bucket_name,
                "status": "created",
                "region": region
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to persist: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_list_iam_users
// ============================================================================

pub struct AwsListIamUsersTool;

#[async_trait]
impl Tool for AwsListIamUsersTool {
    fn name(&self) -> &str {
        "aws_list_iam_users"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List IAM Users")
    }

    fn description(&self) -> &str {
        "List all IAM users in the account."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::List).await;

        let users = read_or_seed(store, context.session_id, IAM_PATH, seed_iam).await;

        ToolExecutionResult::success(json!({"users": users, "total_count": users.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_create_iam_user
// ============================================================================

pub struct AwsCreateIamUserTool;

#[async_trait]
impl Tool for AwsCreateIamUserTool {
    fn name(&self) -> &str {
        "aws_create_iam_user"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Create IAM User")
    }

    fn description(&self) -> &str {
        "Create a new IAM user."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": {"type": "string"},
                "permissions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of permission policies"
                }
            },
            "required": ["username"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::Create).await;

        let username = match arguments.get("username").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: username");
            }
        };
        let permissions: Vec<String> = arguments
            .get("permissions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut users = read_or_seed(store, context.session_id, IAM_PATH, seed_iam).await;

        let user_id = format!(
            "AIDAI{:016x}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64
        );
        let user = IamUser {
            username: username.to_string(),
            user_id: user_id.clone(),
            arn: format!("arn:aws:iam::123456789012:user/{}", username),
            created_at: chrono::Utc::now().to_rfc3339(),
            permissions,
        };
        users.push(user);

        match persist(store, context.session_id, IAM_PATH, &users).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "username": username,
                "user_id": user_id,
                "status": "created"
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to persist: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_list_security_groups
// ============================================================================

pub struct AwsListSecurityGroupsTool;

#[async_trait]
impl Tool for AwsListSecurityGroupsTool {
    fn name(&self) -> &str {
        "aws_list_security_groups"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Security Groups")
    }

    fn description(&self) -> &str {
        "List all security groups with their rules."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::List).await;

        let groups = read_or_seed(store, context.session_id, SG_PATH, seed_security_groups).await;

        ToolExecutionResult::success(
            json!({"security_groups": groups, "total_count": groups.len()}),
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: aws_get_cloudwatch_metrics
// ============================================================================

pub struct AwsGetCloudWatchMetricsTool;

#[async_trait]
impl Tool for AwsGetCloudWatchMetricsTool {
    fn name(&self) -> &str {
        "aws_get_cloudwatch_metrics"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get CloudWatch Metrics")
    }

    fn description(&self) -> &str {
        "Get CloudWatch metrics for a resource (CPU, memory, disk, network)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "metric_name": {
                    "type": "string",
                    "enum": ["CPUUtilization", "MemoryUtilization", "DiskReadOps", "NetworkIn"],
                    "description": "Metric to retrieve"
                },
                "period_minutes": {
                    "type": "integer",
                    "description": "Time period in minutes (default: 60)"
                }
            },
            "required": ["resource_id", "metric_name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = require_file_store!(context);
        simulate_latency(OpKind::Query).await;

        let resource_id = match arguments.get("resource_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: resource_id");
            }
        };
        let metric_name = match arguments.get("metric_name").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: metric_name");
            }
        };

        // Verify the resource actually exists (check EC2 instances)
        let instances = read_or_seed(store, context.session_id, EC2_PATH, seed_ec2).await;
        let instance_exists = instances.iter().any(|i| i.instance_id == resource_id);

        if !instance_exists {
            // Also check RDS
            let databases = read_or_seed(store, context.session_id, RDS_PATH, seed_rds).await;
            let rds_exists = databases.iter().any(|d| d.db_instance_id == resource_id);
            if !rds_exists {
                return ToolExecutionResult::tool_error(format!(
                    "Resource not found: {}",
                    resource_id
                ));
            }
        }

        // Generate metric datapoints — use per-resource profiles for seed instances
        // so auditor sees realistic data (idle instances have low CPU, etc.),
        // with deterministic variation from resource_id hash.
        let hash: u32 = resource_id
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
        let base = metric_profile_base(resource_id, metric_name).unwrap_or(match metric_name {
            "CPUUtilization" => 30.0,
            "MemoryUtilization" => 55.0,
            "DiskReadOps" => 150.0,
            "NetworkIn" => 1024.0 * 50.0,
            _ => 30.0,
        });
        let datapoints: Vec<Value> = (0..12)
            .map(|i| {
                let variation = ((hash.wrapping_add(i * 7)) % 200) as f64 / 10.0 - 10.0;
                let avg = (base + variation * (base / 30.0)).max(0.0);
                json!({
                    "timestamp": chrono::Utc::now() - chrono::Duration::minutes(i as i64 * 5),
                    "average": (avg * 100.0).round() / 100.0,
                    "minimum": ((avg * 0.7).max(0.0) * 100.0).round() / 100.0,
                    "maximum": (avg * 1.3 * 100.0).round() / 100.0
                })
            })
            .collect();

        ToolExecutionResult::success(json!({
            "resource_id": resource_id,
            "metric_name": metric_name,
            "datapoints": datapoints,
            "period": "5 minutes"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}
