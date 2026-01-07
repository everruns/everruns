//! Fake AWS Tools Capability - demo tools for AWS infrastructure management
//!
//! This capability provides mock AWS management tools that store state
//! in the session file system. Perfect for demos and testing.
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

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Fake AWS Tools capability - mock AWS infrastructure management for demos
pub struct FakeAwsCapability;

impl Capability for FakeAwsCapability {
    fn id(&self) -> &str {
        CapabilityId::FAKE_AWS
    }

    fn name(&self) -> &str {
        "Fake AWS Tools"
    }

    fn description(&self) -> &str {
        "Demo capability: AWS infrastructure management tools (EC2, RDS, S3, IAM, CloudWatch). State stored in session filesystem."
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
- /aws/cloudwatch_metrics.json - CloudWatch metrics data"#,
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

// Helper structs for AWS data
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ec2Instance {
    instance_id: String,
    instance_type: String,
    state: String, // running, stopped, terminated
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
// Tool: aws_list_ec2_instances
// ============================================================================

pub struct AwsListEc2InstancesTool;

#[async_trait]
impl Tool for AwsListEc2InstancesTool {
    fn name(&self) -> &str {
        "aws_list_ec2_instances"
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
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let state_filter = arguments.get("state").and_then(|v| v.as_str());

        // Read EC2 instances
        let instances: Vec<Ec2Instance> = match file_store
            .read_file(context.session_id, "/aws/ec2_instances.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(file.content.as_deref().unwrap_or(""))
                .unwrap_or_else(|_| {
                    // Initialize with sample data
                    vec![
                        Ec2Instance {
                            instance_id: "i-0123456789abcdef0".to_string(),
                            instance_type: "t3.medium".to_string(),
                            state: "running".to_string(),
                            availability_zone: "us-east-1a".to_string(),
                            private_ip: "10.0.1.100".to_string(),
                            public_ip: Some("54.123.45.67".to_string()),
                            launch_time: "2025-01-01T10:00:00Z".to_string(),
                            tags: vec![
                                Tag {
                                    key: "Name".to_string(),
                                    value: "web-server-01".to_string(),
                                },
                                Tag {
                                    key: "Environment".to_string(),
                                    value: "production".to_string(),
                                },
                            ],
                        },
                        Ec2Instance {
                            instance_id: "i-0987654321fedcba0".to_string(),
                            instance_type: "t3.large".to_string(),
                            state: "running".to_string(),
                            availability_zone: "us-east-1b".to_string(),
                            private_ip: "10.0.2.100".to_string(),
                            public_ip: Some("54.123.45.68".to_string()),
                            launch_time: "2025-01-01T11:00:00Z".to_string(),
                            tags: vec![Tag {
                                key: "Name".to_string(),
                                value: "api-server-01".to_string(),
                            }],
                        },
                    ]
                }),
            _ => {
                // Create initial data
                let initial = vec![Ec2Instance {
                    instance_id: "i-0123456789abcdef0".to_string(),
                    instance_type: "t3.medium".to_string(),
                    state: "running".to_string(),
                    availability_zone: "us-east-1a".to_string(),
                    private_ip: "10.0.1.100".to_string(),
                    public_ip: Some("54.123.45.67".to_string()),
                    launch_time: chrono::Utc::now().to_rfc3339(),
                    tags: vec![Tag {
                        key: "Name".to_string(),
                        value: "web-server-01".to_string(),
                    }],
                }];

                let content = serde_json::to_string_pretty(&initial).unwrap();
                let _ = file_store
                    .write_file(
                        context.session_id,
                        "/aws/ec2_instances.json",
                        &content,
                        "text",
                    )
                    .await;

                initial
            }
        };

        // Apply filter
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
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let instance_type = match arguments.get("instance_type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: instance_type")
            }
        };

        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolExecutionResult::tool_error("Missing required parameter: name"),
        };

        let availability_zone = arguments
            .get("availability_zone")
            .and_then(|v| v.as_str())
            .unwrap_or("us-east-1a");

        // Read current instances
        let mut instances: Vec<Ec2Instance> = match file_store
            .read_file(context.session_id, "/aws/ec2_instances.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Create new instance
        let instance_id = format!("i-{:016x}", chrono::Utc::now().timestamp() as u64);
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
            state: "running".to_string(),
            availability_zone: availability_zone.to_string(),
            private_ip,
            public_ip: Some(public_ip.clone()),
            launch_time: chrono::Utc::now().to_rfc3339(),
            tags: vec![Tag {
                key: "Name".to_string(),
                value: name.to_string(),
            }],
        };

        instances.push(instance.clone());

        // Save instances
        let content = serde_json::to_string_pretty(&instances).unwrap();
        match file_store
            .write_file(
                context.session_id,
                "/aws/ec2_instances.json",
                &content,
                "text",
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "instance_id": instance_id,
                "state": "running",
                "private_ip": instance.private_ip,
                "public_ip": public_ip,
                "message": "EC2 instance launched successfully"
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
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
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let instance_id = match arguments.get("instance_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: instance_id")
            }
        };

        // Read instances
        let mut instances: Vec<Ec2Instance> = match file_store
            .read_file(context.session_id, "/aws/ec2_instances.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Find and stop instance
        let instance = match instances.iter_mut().find(|i| i.instance_id == instance_id) {
            Some(i) => i,
            None => {
                return ToolExecutionResult::tool_error(format!(
                    "Instance not found: {}",
                    instance_id
                ))
            }
        };

        let old_state = instance.state.clone();
        instance.state = "stopped".to_string();

        // Save updated instances
        let content = serde_json::to_string_pretty(&instances).unwrap();
        match file_store
            .write_file(
                context.session_id,
                "/aws/ec2_instances.json",
                &content,
                "text",
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "instance_id": instance_id,
                "old_state": old_state,
                "new_state": "stopped"
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Remaining tools (simplified implementations)
// ============================================================================

pub struct AwsListRdsDatabasesTool;

#[async_trait]
impl Tool for AwsListRdsDatabasesTool {
    fn name(&self) -> &str {
        "aws_list_rds_databases"
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
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let databases: Vec<RdsDatabase> = match file_store
            .read_file(context.session_id, "/aws/rds_databases.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(file.content.as_deref().unwrap_or(""))
                .unwrap_or_else(|_| {
                    vec![RdsDatabase {
                        db_instance_id: "prod-postgres-01".to_string(),
                        engine: "postgres".to_string(),
                        engine_version: "15.4".to_string(),
                        instance_class: "db.t3.medium".to_string(),
                        status: "available".to_string(),
                        endpoint: "prod-postgres-01.abc123.us-east-1.rds.amazonaws.com".to_string(),
                        port: 5432,
                        storage_gb: 100,
                    }]
                }),
            _ => vec![],
        };

        ToolExecutionResult::success(
            json!({"databases": databases, "total_count": databases.len()}),
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsCreateRdsDatabaseTool;

#[async_trait]
impl Tool for AwsCreateRdsDatabaseTool {
    fn name(&self) -> &str {
        "aws_create_rds_database"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let db_id = arguments
            .get("db_instance_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        ToolExecutionResult::success(json!({
            "db_instance_id": db_id,
            "status": "creating",
            "message": "RDS database creation initiated"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsListS3BucketsTool;

#[async_trait]
impl Tool for AwsListS3BucketsTool {
    fn name(&self) -> &str {
        "aws_list_s3_buckets"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let buckets = vec![
            S3Bucket {
                name: "company-data-backup".to_string(),
                region: "us-east-1".to_string(),
                creation_date: "2024-06-01T00:00:00Z".to_string(),
                versioning_enabled: true,
                encryption_enabled: true,
            },
            S3Bucket {
                name: "static-assets-prod".to_string(),
                region: "us-west-2".to_string(),
                creation_date: "2024-08-15T00:00:00Z".to_string(),
                versioning_enabled: false,
                encryption_enabled: true,
            },
        ];

        ToolExecutionResult::success(json!({"buckets": buckets, "total_count": buckets.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsCreateS3BucketTool;

#[async_trait]
impl Tool for AwsCreateS3BucketTool {
    fn name(&self) -> &str {
        "aws_create_s3_bucket"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let bucket_name = arguments
            .get("bucket_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        ToolExecutionResult::success(json!({
            "bucket_name": bucket_name,
            "status": "created",
            "region": "us-east-1"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsListIamUsersTool;

#[async_trait]
impl Tool for AwsListIamUsersTool {
    fn name(&self) -> &str {
        "aws_list_iam_users"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let users = vec![
            IamUser {
                username: "admin-user".to_string(),
                user_id: "AIDAI23XYZABC123DEF".to_string(),
                arn: "arn:aws:iam::123456789012:user/admin-user".to_string(),
                created_at: "2024-01-15T10:00:00Z".to_string(),
                permissions: vec!["AdministratorAccess".to_string()],
            },
            IamUser {
                username: "developer-user".to_string(),
                user_id: "AIDAI23XYZABC456GHI".to_string(),
                arn: "arn:aws:iam::123456789012:user/developer-user".to_string(),
                created_at: "2024-02-20T14:30:00Z".to_string(),
                permissions: vec!["PowerUserAccess".to_string()],
            },
        ];

        ToolExecutionResult::success(json!({"users": users, "total_count": users.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsCreateIamUserTool;

#[async_trait]
impl Tool for AwsCreateIamUserTool {
    fn name(&self) -> &str {
        "aws_create_iam_user"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let username = arguments
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        ToolExecutionResult::success(json!({
            "username": username,
            "user_id": format!("AIDAI{:016x}", chrono::Utc::now().timestamp()),
            "status": "created"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsListSecurityGroupsTool;

#[async_trait]
impl Tool for AwsListSecurityGroupsTool {
    fn name(&self) -> &str {
        "aws_list_security_groups"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let security_groups = vec![SecurityGroup {
            group_id: "sg-0123456789abcdef0".to_string(),
            group_name: "web-server-sg".to_string(),
            description: "Security group for web servers".to_string(),
            vpc_id: "vpc-abc123".to_string(),
            inbound_rules: vec![
                SecurityRule {
                    protocol: "tcp".to_string(),
                    port_range: "80".to_string(),
                    source: "0.0.0.0/0".to_string(),
                },
                SecurityRule {
                    protocol: "tcp".to_string(),
                    port_range: "443".to_string(),
                    source: "0.0.0.0/0".to_string(),
                },
            ],
            outbound_rules: vec![SecurityRule {
                protocol: "-1".to_string(),
                port_range: "all".to_string(),
                source: "0.0.0.0/0".to_string(),
            }],
        }];

        ToolExecutionResult::success(
            json!({"security_groups": security_groups, "total_count": security_groups.len()}),
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AwsGetCloudWatchMetricsTool;

#[async_trait]
impl Tool for AwsGetCloudWatchMetricsTool {
    fn name(&self) -> &str {
        "aws_get_cloudwatch_metrics"
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
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let resource_id = arguments
            .get("resource_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let metric_name = arguments
            .get("metric_name")
            .and_then(|v| v.as_str())
            .unwrap_or("CPUUtilization");

        // Generate mock metric data
        let datapoints: Vec<Value> = (0..12)
            .map(|i| {
                json!({
                    "timestamp": chrono::Utc::now() - chrono::Duration::minutes(i * 5),
                    "average": 30.0 + (i as f64 * 2.5),
                    "minimum": 20.0,
                    "maximum": 80.0
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
