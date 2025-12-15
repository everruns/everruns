# AWS Production Deployment Specification

## Abstract

This specification defines the AWS infrastructure for deploying the Everruns platform to production. The architecture uses ECS Fargate for container orchestration, RDS PostgreSQL for the database, and self-hosted Temporal for workflow durability. The design optimizes for cost while maintaining production-grade reliability.

This is a temporary implementation that will be replaced with EKS when horizontal scaling is required.

## Requirements

### Functional Requirements

1. Deploy three application services: API, Worker, and UI
2. Deploy self-hosted Temporal server for workflow orchestration
3. Use RDS PostgreSQL as the database for both application and Temporal
4. UI must be publicly accessible via HTTPS
5. API must be publicly accessible via HTTPS at `/api/*` path
6. Pull container images from GitHub Container Registry (ghcr.io)

### Non-Functional Requirements

1. **Cost Optimization**: Target ~$100-120/month for infrastructure
2. **Serverless**: Use Fargate (no EC2 instances to manage)
3. **Fast App Deployment**: ECS service updates should complete in <5 minutes
4. **Security**: All secrets in AWS Secrets Manager, no plaintext credentials

### Constraints

1. Single region deployment (us-east-1)
2. No Multi-AZ for RDS (cost optimization)
3. Single NAT Gateway (cost optimization, acceptable availability tradeoff)
4. Self-hosted Temporal (not Temporal Cloud)

---

## Architecture

### Network Topology

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              VPC: 10.0.0.0/16                               │
│                                                                             │
│  ┌─────────────────────────────┐    ┌─────────────────────────────┐        │
│  │   Public Subnet A           │    │   Public Subnet B           │        │
│  │   10.0.1.0/24 (us-east-1a)  │    │   10.0.2.0/24 (us-east-1b)  │        │
│  │                             │    │                             │        │
│  │   ┌─────────────────────┐   │    │                             │        │
│  │   │   NAT Gateway       │   │    │                             │        │
│  │   └─────────────────────┘   │    │                             │        │
│  │                             │    │                             │        │
│  │   ┌─────────────────────────┴────┴─────────────────────────┐   │        │
│  │   │              Application Load Balancer                 │   │        │
│  │   └─────────────────────────────────────────────────────────┘   │        │
│  └─────────────────────────────┘    └─────────────────────────────┘        │
│                                                                             │
│  ┌─────────────────────────────┐    ┌─────────────────────────────┐        │
│  │   Private Subnet A          │    │   Private Subnet B          │        │
│  │   10.0.10.0/24 (us-east-1a) │    │   10.0.11.0/24 (us-east-1b) │        │
│  │                             │    │                             │        │
│  │   ┌───────────────────┐     │    │   ┌───────────────────┐     │        │
│  │   │ ECS: everruns-api │     │    │   │ ECS: everruns-ui  │     │        │
│  │   └───────────────────┘     │    │   └───────────────────┘     │        │
│  │   ┌───────────────────┐     │    │                             │        │
│  │   │ ECS: everruns-    │     │    │                             │        │
│  │   │      worker       │     │    │                             │        │
│  │   └───────────────────┘     │    │                             │        │
│  │   ┌───────────────────┐     │    │                             │        │
│  │   │ ECS: temporal     │     │    │                             │        │
│  │   └───────────────────┘     │    │                             │        │
│  │                             │    │                             │        │
│  │   ┌───────────────────────────────────────────────────────┐    │        │
│  │   │              RDS PostgreSQL (db.t4g.micro)            │    │        │
│  │   │              (single AZ, in subnet A)                 │    │        │
│  │   └───────────────────────────────────────────────────────┘    │        │
│  └─────────────────────────────┘    └─────────────────────────────┘        │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Service Architecture

```
Internet
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│  ALB (poc.everruns.com)                                     │
│  ├─ HTTPS:443 (ACM certificate)                             │
│  │   ├─ /api/*        → everruns-api:9000                   │
│  │   ├─ /health       → everruns-api:9000                   │
│  │   ├─ /swagger-ui/* → everruns-api:9000                   │
│  │   ├─ /openapi.json → everruns-api:9000                   │
│  │   └─ /* (default)  → everruns-ui:3000                    │
│  └─ HTTP:80 → redirect to HTTPS                             │
└─────────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│  ECS Cluster: everruns-production                           │
│                                                             │
│  ┌─────────────────┐  ┌─────────────────┐                   │
│  │ everruns-api    │  │ everruns-ui     │                   │
│  │ 256 CPU/512 MB  │  │ 256 CPU/512 MB  │                   │
│  │ Port: 9000      │  │ Port: 3000      │                   │
│  │ Replicas: 1     │  │ Replicas: 1     │                   │
│  └────────┬────────┘  └─────────────────┘                   │
│           │                                                 │
│  ┌────────▼────────┐  ┌─────────────────┐                   │
│  │ everruns-worker │  │ temporal        │                   │
│  │ 256 CPU/512 MB  │  │ 512 CPU/1024 MB │                   │
│  │ No port exposed │  │ Port: 7233,8080 │                   │
│  │ Replicas: 1     │  │ Replicas: 1     │                   │
│  └────────┬────────┘  └────────┬────────┘                   │
│           │                    │                            │
│           └─────────┬──────────┘                            │
│                     ▼                                       │
│           ┌─────────────────┐                               │
│           │ RDS PostgreSQL  │                               │
│           │ db.t4g.micro    │                               │
│           │ 20GB gp3        │                               │
│           └─────────────────┘                               │
└─────────────────────────────────────────────────────────────┘
```

---

## Implementation Details

### Directory Structure

```
infra/
└── aws/
    ├── README.md                    # Deployment instructions
    ├── bootstrap/                   # Run FIRST - creates S3 bucket for state
    │   ├── main.tf
    │   ├── variables.tf
    │   ├── outputs.tf
    │   └── terraform.tfvars
    │
    └── production/                  # Main infrastructure
        ├── main.tf                  # Root module composition
        ├── variables.tf             # Input variables
        ├── outputs.tf               # Output values
        ├── terraform.tfvars         # Variable values (git-ignored secrets)
        ├── terraform.tfvars.example # Template for tfvars
        ├── backend.tf               # S3 backend configuration
        │
        └── modules/
            ├── networking/          # VPC, subnets, NAT, security groups
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            ├── database/            # RDS PostgreSQL
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            ├── secrets/             # AWS Secrets Manager
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            ├── ecs-cluster/         # ECS cluster, IAM roles
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            ├── alb/                 # Application Load Balancer
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            ├── acm/                 # SSL certificate
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            ├── ecs-service/         # Reusable ECS service module
            │   ├── main.tf
            │   ├── variables.tf
            │   └── outputs.tf
            │
            └── temporal/            # Temporal server service
                ├── main.tf
                ├── variables.tf
                └── outputs.tf
```

---

### Module Specifications

#### 1. Bootstrap Module (`infra/aws/bootstrap/`)

Creates S3 bucket and DynamoDB table for Terraform state management.

**Resources:**
- `aws_s3_bucket.terraform_state` - Bucket for state files
- `aws_s3_bucket_versioning` - Enable versioning for state history
- `aws_s3_bucket_server_side_encryption_configuration` - AES256 encryption
- `aws_s3_bucket_public_access_block` - Block all public access
- `aws_dynamodb_table.terraform_locks` - State locking table

**Variables:**
```hcl
variable "environment" {
  description = "Environment name"
  type        = string
  default     = "production"
}

variable "region" {
  description = "AWS region"
  type        = string
  default     = "us-east-1"
}
```

**Outputs:**
```hcl
output "state_bucket_name" {
  value = aws_s3_bucket.terraform_state.id
}

output "dynamodb_table_name" {
  value = aws_dynamodb_table.terraform_locks.name
}
```

**Naming Convention:**
- S3 Bucket: `everruns-terraform-state-{environment}`
- DynamoDB Table: `everruns-terraform-locks-{environment}`

---

#### 2. Networking Module (`modules/networking/`)

**Resources:**
- `aws_vpc.main` - VPC with DNS hostnames enabled
- `aws_subnet.public` (x2) - Public subnets in 2 AZs
- `aws_subnet.private` (x2) - Private subnets in 2 AZs
- `aws_internet_gateway.main` - Internet gateway for public subnets
- `aws_eip.nat` - Elastic IP for NAT gateway
- `aws_nat_gateway.main` - Single NAT gateway (cost optimization)
- `aws_route_table.public` - Routes to internet gateway
- `aws_route_table.private` - Routes to NAT gateway
- `aws_route_table_association` - Subnet associations

**Security Groups:**

1. `aws_security_group.alb`:
   - Ingress: 80, 443 from 0.0.0.0/0
   - Egress: All traffic

2. `aws_security_group.ecs`:
   - Ingress: All traffic from ALB security group
   - Ingress: All traffic from self (inter-service communication)
   - Egress: All traffic

3. `aws_security_group.rds`:
   - Ingress: 5432 from ECS security group only
   - Egress: None needed

**Variables:**
```hcl
variable "environment" {
  type = string
}

variable "vpc_cidr" {
  type    = string
  default = "10.0.0.0/16"
}

variable "public_subnet_cidrs" {
  type    = list(string)
  default = ["10.0.1.0/24", "10.0.2.0/24"]
}

variable "private_subnet_cidrs" {
  type    = list(string)
  default = ["10.0.10.0/24", "10.0.11.0/24"]
}

variable "availability_zones" {
  type    = list(string)
  default = ["us-east-1a", "us-east-1b"]
}
```

**Outputs:**
```hcl
output "vpc_id" {}
output "public_subnet_ids" {}
output "private_subnet_ids" {}
output "alb_security_group_id" {}
output "ecs_security_group_id" {}
output "rds_security_group_id" {}
```

---

#### 3. Database Module (`modules/database/`)

**Resources:**
- `aws_db_subnet_group.main` - Subnet group for RDS
- `aws_db_instance.main` - PostgreSQL instance

**Configuration:**
```hcl
resource "aws_db_instance" "main" {
  identifier     = "everruns-${var.environment}"
  engine         = "postgres"
  engine_version = "16.3"
  instance_class = "db.t4g.micro"

  allocated_storage     = 20
  max_allocated_storage = 100  # Autoscaling up to 100GB
  storage_type          = "gp3"
  storage_encrypted     = true

  db_name  = "everruns"
  username = "everruns_admin"
  password = var.db_password  # From secrets module

  vpc_security_group_ids = [var.rds_security_group_id]
  db_subnet_group_name   = aws_db_subnet_group.main.name

  multi_az               = false  # Cost optimization
  publicly_accessible    = false
  skip_final_snapshot    = false
  final_snapshot_identifier = "everruns-${var.environment}-final"

  backup_retention_period = 7
  backup_window          = "03:00-04:00"
  maintenance_window     = "Mon:04:00-Mon:05:00"

  performance_insights_enabled = false  # Cost optimization

  tags = {
    Name        = "everruns-${var.environment}"
    Environment = var.environment
  }
}
```

**Variables:**
```hcl
variable "environment" { type = string }
variable "vpc_id" { type = string }
variable "private_subnet_ids" { type = list(string) }
variable "rds_security_group_id" { type = string }
variable "db_password" {
  type      = string
  sensitive = true
}
```

**Outputs:**
```hcl
output "endpoint" {
  value = aws_db_instance.main.endpoint
}

output "address" {
  value = aws_db_instance.main.address
}

output "port" {
  value = aws_db_instance.main.port
}

output "database_name" {
  value = aws_db_instance.main.db_name
}

output "username" {
  value = aws_db_instance.main.username
}
```

---

#### 4. Secrets Module (`modules/secrets/`)

**Resources:**
- `random_password.db_password` - Auto-generate DB password
- `aws_secretsmanager_secret.db_credentials` - Database credentials
- `aws_secretsmanager_secret_version.db_credentials` - Secret value
- `aws_secretsmanager_secret.openai_api_key` - OpenAI API key (manual input)
- `aws_secretsmanager_secret.github_token` - GitHub token for ghcr.io

**Secret Structure:**

1. Database Credentials (`everruns/{environment}/database`):
```json
{
  "username": "everruns_admin",
  "password": "<auto-generated>",
  "host": "<rds-endpoint>",
  "port": 5432,
  "database": "everruns"
}
```

2. OpenAI API Key (`everruns/{environment}/openai-api-key`):
```json
{
  "api_key": "<manual-input>"
}
```

3. GitHub Token (`everruns/{environment}/github-token`):
```json
{
  "token": "<manual-input>"
}
```

**Variables:**
```hcl
variable "environment" { type = string }
variable "openai_api_key" {
  type      = string
  sensitive = true
}
variable "github_token" {
  type      = string
  sensitive = true
}
variable "db_host" { type = string }
variable "db_port" { type = number }
variable "db_name" { type = string }
variable "db_username" { type = string }
```

**Outputs:**
```hcl
output "db_password" {
  value     = random_password.db_password.result
  sensitive = true
}

output "db_credentials_secret_arn" {
  value = aws_secretsmanager_secret.db_credentials.arn
}

output "openai_api_key_secret_arn" {
  value = aws_secretsmanager_secret.openai_api_key.arn
}

output "github_token_secret_arn" {
  value = aws_secretsmanager_secret.github_token.arn
}
```

---

#### 5. ACM Module (`modules/acm/`)

Creates free SSL certificate with DNS validation.

**Resources:**
- `aws_acm_certificate.main` - SSL certificate request
- `aws_acm_certificate_validation.main` - Certificate validation

**Configuration:**
```hcl
resource "aws_acm_certificate" "main" {
  domain_name       = var.domain_name  # poc.everruns.com
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }

  tags = {
    Name        = "everruns-${var.environment}"
    Environment = var.environment
  }
}
```

**Variables:**
```hcl
variable "environment" { type = string }
variable "domain_name" {
  type        = string
  description = "Domain name for SSL certificate (e.g., poc.everruns.com)"
}
```

**Outputs:**
```hcl
output "certificate_arn" {
  value = aws_acm_certificate.main.arn
}

output "domain_validation_options" {
  description = "DNS records to create for validation"
  value       = aws_acm_certificate.main.domain_validation_options
}
```

**Manual Step Required:**
After running Terraform, you must create a CNAME record in your DNS provider:
```
Name:  _<hash>.poc.everruns.com
Value: _<hash>.acm-validations.aws.
```

---

#### 6. ECS Cluster Module (`modules/ecs-cluster/`)

**Resources:**
- `aws_ecs_cluster.main` - ECS cluster
- `aws_cloudwatch_log_group.ecs` - Log group for all services
- `aws_iam_role.ecs_execution` - Task execution role
- `aws_iam_role.ecs_task` - Task role for application
- `aws_iam_policy.secrets_access` - Policy to read secrets
- `aws_service_discovery_private_dns_namespace.main` - Service discovery

**IAM Execution Role Policy:**
```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "ecr:GetAuthorizationToken",
        "ecr:BatchCheckLayerAvailability",
        "ecr:GetDownloadUrlForLayer",
        "ecr:BatchGetImage",
        "logs:CreateLogStream",
        "logs:PutLogEvents"
      ],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": [
        "secretsmanager:GetSecretValue"
      ],
      "Resource": [
        "arn:aws:secretsmanager:*:*:secret:everruns/*"
      ]
    }
  ]
}
```

**Variables:**
```hcl
variable "environment" { type = string }
variable "vpc_id" { type = string }
```

**Outputs:**
```hcl
output "cluster_id" {}
output "cluster_name" {}
output "execution_role_arn" {}
output "task_role_arn" {}
output "log_group_name" {}
output "service_discovery_namespace_id" {}
output "service_discovery_namespace_name" {}
```

---

#### 7. ALB Module (`modules/alb/`)

**Resources:**
- `aws_lb.main` - Application Load Balancer
- `aws_lb_listener.http` - HTTP listener (redirect to HTTPS)
- `aws_lb_listener.https` - HTTPS listener
- `aws_lb_target_group.api` - Target group for API
- `aws_lb_target_group.ui` - Target group for UI
- `aws_lb_listener_rule.api` - Route /api/* to API
- `aws_lb_listener_rule.swagger` - Route /swagger-ui/* to API
- `aws_lb_listener_rule.openapi` - Route /openapi.json to API
- `aws_lb_listener_rule.health` - Route /health to API

**ALB Configuration:**
```hcl
resource "aws_lb" "main" {
  name               = "everruns-${var.environment}"
  internal           = false
  load_balancer_type = "application"
  security_groups    = [var.alb_security_group_id]
  subnets            = var.public_subnet_ids

  enable_deletion_protection = true

  tags = {
    Name        = "everruns-${var.environment}"
    Environment = var.environment
  }
}
```

**Listener Rules (priority order):**
1. `/api/*` → API target group
2. `/health` → API target group
3. `/swagger-ui/*` → API target group
4. `/openapi.json` → API target group
5. Default → UI target group

**Health Checks:**
- API: `GET /health`, expected 200, interval 30s
- UI: `GET /`, expected 200, interval 30s

**Variables:**
```hcl
variable "environment" { type = string }
variable "vpc_id" { type = string }
variable "public_subnet_ids" { type = list(string) }
variable "alb_security_group_id" { type = string }
variable "certificate_arn" { type = string }
```

**Outputs:**
```hcl
output "alb_arn" {}
output "alb_dns_name" {}
output "alb_zone_id" {}
output "https_listener_arn" {}
output "api_target_group_arn" {}
output "ui_target_group_arn" {}
```

---

#### 8. ECS Service Module (`modules/ecs-service/`)

Reusable module for deploying ECS services.

**Resources:**
- `aws_ecs_task_definition.main` - Task definition
- `aws_ecs_service.main` - ECS service
- `aws_service_discovery_service.main` - Service discovery (optional)

**Task Definition Template:**
```hcl
resource "aws_ecs_task_definition" "main" {
  family                   = "${var.service_name}-${var.environment}"
  network_mode             = "awsvpc"
  requires_compatibilities = ["FARGATE"]
  cpu                      = var.cpu
  memory                   = var.memory
  execution_role_arn       = var.execution_role_arn
  task_role_arn            = var.task_role_arn

  container_definitions = jsonencode([
    {
      name      = var.service_name
      image     = var.image
      essential = true

      portMappings = var.port != null ? [
        {
          containerPort = var.port
          hostPort      = var.port
          protocol      = "tcp"
        }
      ] : []

      environment = var.environment_variables

      secrets = var.secrets

      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = var.log_group_name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = var.service_name
        }
      }

      healthCheck = var.health_check_command != null ? {
        command     = var.health_check_command
        interval    = 30
        timeout     = 5
        retries     = 3
        startPeriod = 60
      } : null
    }
  ])
}
```

**Variables:**
```hcl
variable "environment" { type = string }
variable "service_name" { type = string }
variable "cluster_id" { type = string }
variable "image" { type = string }
variable "cpu" {
  type    = number
  default = 256
}
variable "memory" {
  type    = number
  default = 512
}
variable "port" {
  type    = number
  default = null
}
variable "desired_count" {
  type    = number
  default = 1
}
variable "environment_variables" {
  type    = list(object({ name = string, value = string }))
  default = []
}
variable "secrets" {
  type    = list(object({ name = string, valueFrom = string }))
  default = []
}
variable "execution_role_arn" { type = string }
variable "task_role_arn" { type = string }
variable "log_group_name" { type = string }
variable "region" { type = string }
variable "subnet_ids" { type = list(string) }
variable "security_group_ids" { type = list(string) }
variable "target_group_arn" {
  type    = string
  default = null
}
variable "health_check_command" {
  type    = list(string)
  default = null
}
variable "enable_service_discovery" {
  type    = bool
  default = false
}
variable "service_discovery_namespace_id" {
  type    = string
  default = null
}
```

**Outputs:**
```hcl
output "service_name" {}
output "task_definition_arn" {}
output "service_discovery_name" {}
```

---

#### 9. Temporal Module (`modules/temporal/`)

Specialized module for Temporal server deployment.

**Image:** `temporalio/auto-setup:1.24.2`

This image automatically:
- Creates the Temporal database schema
- Sets up the default namespace
- Runs both frontend and worker

**Task Definition:**
```hcl
container_definitions = jsonencode([
  {
    name      = "temporal"
    image     = "temporalio/auto-setup:1.24.2"
    essential = true

    portMappings = [
      { containerPort = 7233, hostPort = 7233, protocol = "tcp" },
      { containerPort = 8080, hostPort = 8080, protocol = "tcp" }
    ]

    environment = [
      { name = "DB", value = "postgresql" },
      { name = "DB_PORT", value = "5432" },
      { name = "POSTGRES_SEEDS", value = var.db_host },
      { name = "DBNAME", value = "temporal" },
      { name = "VISIBILITY_DBNAME", value = "temporal_visibility" }
    ]

    secrets = [
      { name = "POSTGRES_USER", valueFrom = "${var.db_credentials_secret_arn}:username::" },
      { name = "POSTGRES_PWD", valueFrom = "${var.db_credentials_secret_arn}:password::" }
    ]

    logConfiguration = {
      logDriver = "awslogs"
      options = {
        "awslogs-group"         = var.log_group_name
        "awslogs-region"        = var.region
        "awslogs-stream-prefix" = "temporal"
      }
    }
  }
])
```

**Service Discovery:**
Register as `temporal.everruns.local` for internal access.

**Note:** Temporal creates its own databases (`temporal`, `temporal_visibility`) in the same PostgreSQL instance.

---

### Main Module Composition (`production/main.tf`)

```hcl
terraform {
  required_version = ">= 1.5.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.5"
    }
  }
}

provider "aws" {
  region = var.region

  default_tags {
    tags = {
      Project     = "everruns"
      Environment = var.environment
      ManagedBy   = "terraform"
    }
  }
}

# 1. Networking
module "networking" {
  source      = "./modules/networking"
  environment = var.environment
}

# 2. Secrets (creates DB password)
module "secrets" {
  source         = "./modules/secrets"
  environment    = var.environment
  openai_api_key = var.openai_api_key
  github_token   = var.github_token
  db_host        = module.database.address
  db_port        = module.database.port
  db_name        = module.database.database_name
  db_username    = module.database.username

  depends_on = [module.database]
}

# 3. Database
module "database" {
  source                = "./modules/database"
  environment           = var.environment
  vpc_id                = module.networking.vpc_id
  private_subnet_ids    = module.networking.private_subnet_ids
  rds_security_group_id = module.networking.rds_security_group_id
  db_password           = module.secrets.db_password
}

# 4. ACM Certificate
module "acm" {
  source      = "./modules/acm"
  environment = var.environment
  domain_name = var.domain_name
}

# 5. ECS Cluster
module "ecs_cluster" {
  source      = "./modules/ecs-cluster"
  environment = var.environment
  vpc_id      = module.networking.vpc_id
}

# 6. ALB
module "alb" {
  source                = "./modules/alb"
  environment           = var.environment
  vpc_id                = module.networking.vpc_id
  public_subnet_ids     = module.networking.public_subnet_ids
  alb_security_group_id = module.networking.alb_security_group_id
  certificate_arn       = module.acm.certificate_arn
}

# 7. Temporal Service
module "temporal" {
  source                         = "./modules/temporal"
  environment                    = var.environment
  cluster_id                     = module.ecs_cluster.cluster_id
  execution_role_arn             = module.ecs_cluster.execution_role_arn
  task_role_arn                  = module.ecs_cluster.task_role_arn
  log_group_name                 = module.ecs_cluster.log_group_name
  region                         = var.region
  subnet_ids                     = module.networking.private_subnet_ids
  security_group_ids             = [module.networking.ecs_security_group_id]
  db_host                        = module.database.address
  db_credentials_secret_arn      = module.secrets.db_credentials_secret_arn
  service_discovery_namespace_id = module.ecs_cluster.service_discovery_namespace_id
}

# 8. API Service
module "api" {
  source             = "./modules/ecs-service"
  environment        = var.environment
  service_name       = "everruns-api"
  cluster_id         = module.ecs_cluster.cluster_id
  image              = "ghcr.io/everruns/everruns-api:${var.image_tag}"
  cpu                = 256
  memory             = 512
  port               = 9000
  desired_count      = 1
  execution_role_arn = module.ecs_cluster.execution_role_arn
  task_role_arn      = module.ecs_cluster.task_role_arn
  log_group_name     = module.ecs_cluster.log_group_name
  region             = var.region
  subnet_ids         = module.networking.private_subnet_ids
  security_group_ids = [module.networking.ecs_security_group_id]
  target_group_arn   = module.alb.api_target_group_arn

  environment_variables = [
    { name = "HOST", value = "0.0.0.0" },
    { name = "PORT", value = "9000" },
    { name = "RUST_LOG", value = "info" },
    { name = "AGENT_RUNNER_MODE", value = "temporal" },
    { name = "TEMPORAL_ADDRESS", value = "temporal.everruns.local:7233" },
    { name = "TEMPORAL_NAMESPACE", value = "default" },
    { name = "TEMPORAL_TASK_QUEUE", value = "everruns-agent-runs" }
  ]

  secrets = [
    {
      name      = "DATABASE_URL"
      valueFrom = "${module.secrets.db_credentials_secret_arn}:connection_string::"
    },
    {
      name      = "OPENAI_API_KEY"
      valueFrom = "${module.secrets.openai_api_key_secret_arn}:api_key::"
    }
  ]

  health_check_command = ["CMD-SHELL", "curl -f http://localhost:9000/health || exit 1"]

  depends_on = [module.temporal]
}

# 9. Worker Service
module "worker" {
  source             = "./modules/ecs-service"
  environment        = var.environment
  service_name       = "everruns-worker"
  cluster_id         = module.ecs_cluster.cluster_id
  image              = "ghcr.io/everruns/everruns-worker:${var.image_tag}"
  cpu                = 256
  memory             = 512
  port               = null  # No port exposed
  desired_count      = 1
  execution_role_arn = module.ecs_cluster.execution_role_arn
  task_role_arn      = module.ecs_cluster.task_role_arn
  log_group_name     = module.ecs_cluster.log_group_name
  region             = var.region
  subnet_ids         = module.networking.private_subnet_ids
  security_group_ids = [module.networking.ecs_security_group_id]

  environment_variables = [
    { name = "RUST_LOG", value = "info" },
    { name = "AGENT_RUNNER_MODE", value = "temporal" },
    { name = "TEMPORAL_ADDRESS", value = "temporal.everruns.local:7233" },
    { name = "TEMPORAL_NAMESPACE", value = "default" },
    { name = "TEMPORAL_TASK_QUEUE", value = "everruns-agent-runs" }
  ]

  secrets = [
    {
      name      = "DATABASE_URL"
      valueFrom = "${module.secrets.db_credentials_secret_arn}:connection_string::"
    },
    {
      name      = "OPENAI_API_KEY"
      valueFrom = "${module.secrets.openai_api_key_secret_arn}:api_key::"
    }
  ]

  depends_on = [module.temporal]
}

# 10. UI Service
module "ui" {
  source             = "./modules/ecs-service"
  environment        = var.environment
  service_name       = "everruns-ui"
  cluster_id         = module.ecs_cluster.cluster_id
  image              = "ghcr.io/everruns/everruns-ui:${var.image_tag}"
  cpu                = 256
  memory             = 512
  port               = 3000
  desired_count      = 1
  execution_role_arn = module.ecs_cluster.execution_role_arn
  task_role_arn      = module.ecs_cluster.task_role_arn
  log_group_name     = module.ecs_cluster.log_group_name
  region             = var.region
  subnet_ids         = module.networking.private_subnet_ids
  security_group_ids = [module.networking.ecs_security_group_id]
  target_group_arn   = module.alb.ui_target_group_arn

  environment_variables = [
    { name = "NODE_ENV", value = "production" },
    { name = "NEXT_PUBLIC_API_BASE_URL", value = "https://${var.domain_name}" }
  ]
}
```

---

### Variables (`production/variables.tf`)

```hcl
variable "environment" {
  description = "Environment name"
  type        = string
  default     = "production"
}

variable "region" {
  description = "AWS region"
  type        = string
  default     = "us-east-1"
}

variable "domain_name" {
  description = "Domain name for the application"
  type        = string
  default     = "poc.everruns.com"
}

variable "image_tag" {
  description = "Docker image tag to deploy"
  type        = string
  default     = "latest"
}

variable "openai_api_key" {
  description = "OpenAI API key"
  type        = string
  sensitive   = true
}

variable "github_token" {
  description = "GitHub token for pulling images from ghcr.io"
  type        = string
  sensitive   = true
}
```

---

### Outputs (`production/outputs.tf`)

```hcl
output "alb_dns_name" {
  description = "ALB DNS name (for CNAME record)"
  value       = module.alb.alb_dns_name
}

output "api_url" {
  description = "API URL"
  value       = "https://${var.domain_name}/api"
}

output "ui_url" {
  description = "UI URL"
  value       = "https://${var.domain_name}"
}

output "rds_endpoint" {
  description = "RDS endpoint"
  value       = module.database.endpoint
  sensitive   = true
}

output "acm_validation_records" {
  description = "DNS records needed for ACM certificate validation"
  value       = module.acm.domain_validation_options
}
```

---

## Deployment Procedure

### Prerequisites

1. AWS CLI configured with appropriate credentials
2. Terraform >= 1.5.0 installed
3. Domain access to everruns.com for DNS validation
4. OpenAI API key
5. GitHub personal access token with `read:packages` scope

### Step 1: Bootstrap (One-time)

```bash
cd infra/aws/bootstrap

# Initialize and apply
terraform init
terraform apply

# Save outputs for next step
terraform output
```

### Step 2: Configure Backend

Create `infra/aws/production/backend.tf`:
```hcl
terraform {
  backend "s3" {
    bucket         = "everruns-terraform-state-production"
    key            = "production/terraform.tfstate"
    region         = "us-east-1"
    dynamodb_table = "everruns-terraform-locks-production"
    encrypt        = true
  }
}
```

### Step 3: Create terraform.tfvars

Create `infra/aws/production/terraform.tfvars`:
```hcl
environment    = "production"
region         = "us-east-1"
domain_name    = "poc.everruns.com"
image_tag      = "latest"
openai_api_key = "sk-..."  # Your OpenAI API key
github_token   = "ghp_..."  # GitHub PAT with read:packages scope
```

**Important:** Add `terraform.tfvars` to `.gitignore` - it contains secrets!

### Step 4: Deploy Infrastructure

```bash
cd infra/aws/production

# Initialize
terraform init

# Plan
terraform plan -out=tfplan

# Apply
terraform apply tfplan
```

### Step 5: DNS Configuration

After Terraform apply, you'll see `acm_validation_records` output. Create CNAME record in your DNS provider:

```
Type: CNAME
Name: _<hash>.poc.everruns.com
Value: _<hash>.acm-validations.aws.
TTL: 300
```

Also create CNAME for the application:
```
Type: CNAME
Name: poc.everruns.com
Value: <alb_dns_name from output>
TTL: 300
```

Wait for certificate validation (usually 5-30 minutes).

### Step 6: Run Database Migrations

Migrations run automatically through the API service on startup (via sqlx embedded migrations).

Alternatively, run manually:
```bash
# Create a one-time ECS task for migrations
aws ecs run-task \
  --cluster everruns-production \
  --task-definition everruns-api-production \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[subnet-xxx],securityGroups=[sg-xxx],assignPublicIp=DISABLED}" \
  --overrides '{"containerOverrides":[{"name":"everruns-api","command":["sqlx","migrate","run","--source","crates/everruns-storage/migrations"]}]}'
```

### Step 7: Verify Deployment

```bash
# Check ECS services
aws ecs list-services --cluster everruns-production

# Check service status
aws ecs describe-services --cluster everruns-production --services everruns-api-production

# Test health endpoint
curl https://poc.everruns.com/health

# Test API
curl https://poc.everruns.com/api/v1/agents
```

---

## Updating Deployments

### Deploy New Image Version

```bash
# Option 1: Update image tag in tfvars and apply
terraform apply -var="image_tag=abc123"

# Option 2: Force new deployment with same tag
aws ecs update-service \
  --cluster everruns-production \
  --service everruns-api-production \
  --force-new-deployment
```

### Rollback

```bash
# Update to previous image tag
terraform apply -var="image_tag=previous-sha"
```

---

## Cost Optimization Notes

1. **RDS**: Using db.t4g.micro (smallest Graviton instance) instead of Aurora
2. **NAT Gateway**: Single NAT instead of one per AZ
3. **Fargate**: Smallest viable task sizes (256 CPU / 512 MB)
4. **ALB**: Single ALB with path-based routing instead of multiple
5. **No Multi-AZ**: RDS runs in single AZ (acceptable for POC)

### Future Scaling (EKS Migration)

When migrating to EKS:
1. Export RDS data
2. Deploy EKS cluster
3. Use same RDS instance (just update security groups)
4. Deploy workloads via Kubernetes manifests
5. Switch ALB target groups or use Ingress controller
6. Decommission ECS cluster

---

## Troubleshooting

### ECS Tasks Not Starting

```bash
# Check task events
aws ecs describe-services --cluster everruns-production --services everruns-api-production

# Check stopped tasks
aws ecs list-tasks --cluster everruns-production --desired-status STOPPED
aws ecs describe-tasks --cluster everruns-production --tasks <task-arn>
```

### Database Connection Issues

```bash
# Check security groups allow traffic
aws ec2 describe-security-groups --group-ids <rds-sg-id>

# Test from ECS task
aws ecs execute-command --cluster everruns-production \
  --task <task-id> --container everruns-api \
  --interactive --command "/bin/sh"
```

### Certificate Not Validating

1. Verify CNAME record is correct
2. Check ACM console for validation status
3. DNS propagation can take up to 48 hours (usually much faster)

### Image Pull Failures

```bash
# Verify GitHub token has correct permissions
# Token needs: read:packages scope

# Check secret in Secrets Manager
aws secretsmanager get-secret-value --secret-id everruns/production/github-token
```
