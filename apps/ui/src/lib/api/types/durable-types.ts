// Durable Execution types

// ============================================
// Durable Execution types
// ============================================

/** Worker status */
export type WorkerStatus = "active" | "draining" | "stopped" | "stale";

/** Workflow status */
export type WorkflowStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

/** Task status in the queue */
export type TaskStatus = "pending" | "claimed" | "completed" | "failed" | "dead" | "cancelled";

/** Circuit breaker state */
export type CircuitBreakerState = "closed" | "open" | "half_open";

/** System health status */
export type HealthStatus = "healthy" | "degraded" | "unhealthy";

/** Worker information */
export interface DurableWorker {
  id: string;
  worker_group: string;
  activity_types: string[];
  max_concurrency: number;
  current_load: number;
  status: WorkerStatus;
  accepting_tasks: boolean;
  backpressure_reason?: string;
  started_at: string;
  last_heartbeat_at: string;
  hostname?: string;
  version?: string;
  metadata?: Record<string, unknown>;
  // Live stats (optional - may not be tracked)
  tasks_completed?: number;
  tasks_failed?: number;
  avg_task_duration_ms?: number;
}

/** Workers list summary */
export interface WorkersSummary {
  active: number;
  draining: number;
  stopped: number;
  total_capacity: number;
  total_load: number;
}

/** Workers list response */
export interface WorkersResponse {
  data: DurableWorker[];
  total: number;
  summary?: WorkersSummary;
}

/** Workflow instance */
export interface DurableWorkflow {
  id: string;
  workflow_type: string;
  status: WorkflowStatus;
  input: Record<string, unknown>;
  result?: Record<string, unknown>;
  error?: Record<string, unknown>;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  // Optional linked session
  session_id?: string;
  agent_id?: string;
}

/** Workflow event */
export interface WorkflowEvent {
  id: number;
  workflow_id: string;
  sequence_num: number;
  event_type: string;
  event_data: Record<string, unknown>;
  created_at: string;
}

/** Task in the queue */
export interface DurableTask {
  id: string;
  workflow_id?: string;
  activity_id: string;
  activity_type: string;
  status: TaskStatus;
  priority: number;
  scheduled_at: string;
  visible_at: string;
  claimed_by?: string;
  claimed_at?: string;
  heartbeat_at?: string;
  attempt: number;
  max_attempts: number;
  last_error?: string;
  created_at: string;
}

/** Activity type statistics */
export interface ActivityTypeStats {
  pending: number;
  claimed: number;
  completed_last_hour: number;
  failed_last_hour: number;
  avg_duration_ms: number;
  p99_duration_ms: number;
}

/** Task queue statistics */
export interface TaskQueueStats {
  by_activity_type: Record<string, ActivityTypeStats>;
  by_priority: Record<number, number>;
  oldest_pending_task_age_ms: number;
  avg_schedule_to_start_ms: number;
  avg_execution_time_ms: number;
}

/** Dead letter queue entry */
export interface DlqEntry {
  id: string;
  original_task_id: string;
  workflow_id?: string;
  activity_id: string;
  activity_type: string;
  input: Record<string, unknown>;
  attempts: number;
  last_error: string;
  error_history: string[];
  dead_at: string;
  requeued_at?: string;
  requeue_count: number;
}

/** Circuit breaker information */
export interface CircuitBreaker {
  key: string;
  state: CircuitBreakerState;
  failure_count: number;
  success_count: number;
  last_failure_at?: string;
  opened_at?: string;
  half_open_at?: string;
  updated_at: string;
}

/** Durable system health */
export interface DurableSystemHealth {
  status: HealthStatus;
  // Workers
  total_workers: number;
  active_workers: number;
  workers_accepting: number;
  total_capacity: number;
  current_load: number;
  load_percentage: number;
  // Task queue
  pending_tasks: number;
  claimed_tasks: number;
  completed_tasks: number;
  failed_tasks: number;
  started_tasks: number;
  queue_depth_by_type?: Record<string, number>;
  // Workflows
  running_workflows: number;
  pending_workflows: number;
  completed_workflows: number;
  failed_workflows: number;
  started_workflows: number;
  // DLQ
  dlq_size: number;
  // Circuit breakers (optional - computed from circuit-breakers endpoint if needed)
  open_circuit_breakers?: string[];
}

/** Workflows list response */
export interface WorkflowsResponse {
  data: DurableWorkflow[];
  total: number;
}

/** Tasks list response */
export interface TasksResponse {
  data: DurableTask[];
  total: number;
}

/** DLQ list response */
export interface DlqResponse {
  data: DlqEntry[];
  total: number;
}

/** Workflow events list response */
export interface WorkflowEventsResponse {
  data: WorkflowEvent[];
  total: number;
}

/** Circuit breakers list response */
export interface CircuitBreakersResponse {
  data: CircuitBreaker[];
  total: number;
}

/** Single metrics data point (timestamped health snapshot) */
export interface MetricsPoint {
  timestamp: string;
  running_workflows: number;
  pending_workflows: number;
  pending_tasks: number;
  claimed_tasks: number;
  active_workers: number;
  load_percentage: number;
  dlq_size: number;
  tasks_completed_total: number;
  tasks_failed_total: number;
  tasks_started_total: number;
  workflows_completed_total: number;
  workflows_failed_total: number;
  workflows_started_total: number;
}

/** Metrics time series response */
export interface MetricsTimeSeriesResponse {
  points: MetricsPoint[];
  resolution_seconds: number;
}

// ============================================
// Durable Schedule types
// ============================================

/** Schedule target type */
export type ScheduleTargetType = "workflow" | "activity";

/** Schedule execution status */
export type ScheduleExecutionStatus = "pending" | "running" | "completed" | "failed" | "skipped";

/** Schedule target configuration */
export interface ScheduleTarget {
  type: ScheduleTargetType;
  name: string;
  input: Record<string, unknown>;
}

/** Scheduled task */
export interface DurableSchedule {
  id: string;
  name: string;
  description?: string;
  cron_expression: string;
  timezone: string;
  target: ScheduleTarget;
  enabled: boolean;
  max_concurrent?: number;
  catch_up_missed: boolean;
  max_catch_up?: number;
  retry_policy?: Record<string, unknown>;
  last_triggered_at?: string;
  next_trigger_at?: string;
  created_at: string;
  updated_at: string;
}

/** Schedule execution record */
export interface ScheduleExecution {
  id: string;
  schedule_id: string;
  scheduled_at: string;
  started_at: string;
  completed_at?: string;
  status: ScheduleExecutionStatus;
  workflow_id?: string;
  task_id?: string;
  error?: string;
  duration_ms?: number;
  created_at: string;
}

/** Schedule statistics */
export interface ScheduleStats {
  total_executions: number;
  successful_executions: number;
  failed_executions: number;
  skipped_executions: number;
  avg_duration_ms?: number;
  last_execution_status?: ScheduleExecutionStatus;
}

/** Request to create a schedule */
export interface CreateScheduleRequest {
  name: string;
  description?: string;
  cron_expression: string;
  timezone?: string;
  target: ScheduleTarget;
  enabled?: boolean;
  max_concurrent?: number;
  catch_up_missed?: boolean;
  max_catch_up?: number;
  retry_policy?: Record<string, unknown>;
}

/** Request to update a schedule */
export interface UpdateScheduleRequest {
  description?: string;
  cron_expression?: string;
  timezone?: string;
  target?: ScheduleTarget;
  enabled?: boolean;
  max_concurrent?: number;
  catch_up_missed?: boolean;
  max_catch_up?: number;
  retry_policy?: Record<string, unknown>;
}

/** Schedules list response */
export interface SchedulesResponse {
  data: DurableSchedule[];
  total: number;
}

/** Schedule executions list response */
export interface ScheduleExecutionsResponse {
  data: ScheduleExecution[];
  total: number;
}

/** Manual trigger response */
export interface TriggerResponse {
  execution_id: string;
}
