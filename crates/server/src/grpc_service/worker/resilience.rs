//! Circuit breaker state.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_check_circuit_breaker(
        &self,
        request: Request<CheckCircuitBreakerRequest>,
    ) -> Result<Response<CheckCircuitBreakerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        // Create a circuit breaker instance for this key
        let config = CircuitBreakerConfig::default();
        let store_dyn: Arc<dyn WorkflowEventStore> = store.clone();
        let breaker = DistributedCircuitBreaker::new(req.key.clone(), config, store_dyn);

        // Try to get a permit (this checks if the circuit allows the call)
        match breaker.allow().await {
            Ok(_permit) => {
                // Permit granted - get current state
                let state = breaker.state().await.unwrap_or(CircuitState::Closed);
                Ok(Response::new(CheckCircuitBreakerResponse {
                    allowed: true,
                    state: circuit_state_to_proto(state).into(),
                }))
            }
            Err(e) => {
                // Circuit is open or half-open
                let state = match e {
                    everruns_durable::CircuitBreakerError::Open => CircuitState::Open,
                    _ => {
                        tracing::error!("Circuit breaker error: {}", e);
                        CircuitState::Closed
                    }
                };
                Ok(Response::new(CheckCircuitBreakerResponse {
                    allowed: false,
                    state: circuit_state_to_proto(state).into(),
                }))
            }
        }
    }

    pub(crate) async fn handle_record_circuit_breaker_success(
        &self,
        request: Request<RecordCircuitBreakerSuccessRequest>,
    ) -> Result<Response<RecordCircuitBreakerSuccessResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        // Create a circuit breaker instance and record success
        let config = CircuitBreakerConfig::default();
        let store_dyn: Arc<dyn WorkflowEventStore> = store.clone();
        let breaker = DistributedCircuitBreaker::new(req.key.clone(), config, store_dyn);

        // Get permit and record success
        if let Ok(permit) = breaker.allow().await {
            permit.success().await.map_err(|e| {
                tracing::error!("Failed to record circuit breaker success: {}", e);
                Status::internal("Failed to record success")
            })?;
        }

        let state = breaker.state().await.unwrap_or(CircuitState::Closed);
        Ok(Response::new(RecordCircuitBreakerSuccessResponse {
            state: circuit_state_to_proto(state).into(),
        }))
    }

    pub(crate) async fn handle_record_circuit_breaker_failure(
        &self,
        request: Request<RecordCircuitBreakerFailureRequest>,
    ) -> Result<Response<RecordCircuitBreakerFailureResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        // Create a circuit breaker instance and record failure
        let config = CircuitBreakerConfig::default();
        let store_dyn: Arc<dyn WorkflowEventStore> = store.clone();
        let breaker = DistributedCircuitBreaker::new(req.key.clone(), config, store_dyn);

        // Get the state before recording failure
        let state_before = breaker.state().await.unwrap_or(CircuitState::Closed);

        // Get permit and record failure
        if let Ok(permit) = breaker.allow().await {
            permit.failure().await.map_err(|e| {
                tracing::error!("Failed to record circuit breaker failure: {}", e);
                Status::internal("Failed to record failure")
            })?;
        }

        // Get the state after recording failure
        let state_after = breaker.state().await.unwrap_or(CircuitState::Closed);
        let circuit_opened =
            state_before != CircuitState::Open && state_after == CircuitState::Open;

        if circuit_opened {
            tracing::warn!(key = %req.key, "Circuit breaker opened");
        }

        Ok(Response::new(RecordCircuitBreakerFailureResponse {
            state: circuit_state_to_proto(state_after).into(),
            circuit_opened,
        }))
    }
}
