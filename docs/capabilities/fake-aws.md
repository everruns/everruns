---
title: Fake AWS
description: Demo capability with simulated AWS infrastructure management tools
---

| | |
|---|---|
| **ID** | `fake_aws` |
| **Category** | Demo |
| **Risk** | Low |
| **Features** | None |
| **Dependencies** | None |

Simulated AWS infrastructure management tools for testing and demonstrations. Covers EC2, RDS, S3, IAM, security groups, and CloudWatch. State is persisted in the session filesystem under `/aws/`. Simulates realistic API latency (configurable via `FAKE_AWS_LATENCY_MS`).

## Tools

| Tool | Description |
|---|---|
| `aws_list_ec2_instances` | List EC2 instances |
| `aws_create_ec2_instance` | Launch a new EC2 instance |
| `aws_stop_ec2_instance` | Stop an EC2 instance |
| `aws_list_rds_databases` | List RDS databases |
| `aws_create_rds_database` | Create an RDS database |
| `aws_list_s3_buckets` | List S3 buckets |
| `aws_create_s3_bucket` | Create an S3 bucket |
| `aws_list_iam_users` | List IAM users |
| `aws_create_iam_user` | Create an IAM user |
| `aws_list_security_groups` | List security groups |
| `aws_get_cloudwatch_metrics` | Get CloudWatch metrics |

## Use Cases

- **DevOps agent demos** — showcase agents managing cloud infrastructure
- **Testing multi-service workflows** — validate agents coordinating across AWS services
- **Training** — practice AWS operations without real cloud access or cost

## Example

```
User: Set up a web server with a database

Agent:
  → aws_create_ec2_instance({ instance_type: "t3.medium", ami: "ami-web-server" })
  ← { instance_id: "i-abc123", status: "running" }
  → aws_create_rds_database({ engine: "postgres", instance_class: "db.t3.micro" })
  ← { db_identifier: "db-xyz789", status: "available", endpoint: "db-xyz789.rds.amazonaws.com" }
  → aws_create_s3_bucket({ bucket_name: "app-assets-prod" })
  ← { bucket: "app-assets-prod", status: "created" }
```

## See Also

- [Fake Warehouse](/capabilities/fake-warehouse/) — simulated warehouse operations
- [Fake CRM](/capabilities/fake-crm/) — simulated customer management
- [Capabilities Overview](/capabilities/)
