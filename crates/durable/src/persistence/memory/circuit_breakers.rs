//! Implementation of [`crate::CircuitBreakers`].

use super::*;

#[async_trait]
impl CircuitBreakers for InMemoryWorkflowEventStore {
    async fn create_circuit_breaker(
        &self,
        key: &str,
        _config: &crate::reliability::CircuitBreakerConfig,
    ) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        breakers.insert(
            key.to_string(),
            CircuitBreakerMemState {
                state: crate::reliability::CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                opened_at: None,
            },
        );
        Ok(())
    }

    async fn get_circuit_breaker(
        &self,
        key: &str,
    ) -> Result<Option<CircuitBreakerState>, StoreError> {
        let breakers = self.circuit_breakers.read();
        Ok(breakers.get(key).map(|b| CircuitBreakerState {
            key: key.to_string(),
            state: b.state,
            failure_count: b.failure_count,
            success_count: b.success_count,
            last_failure_at: None,
            opened_at: b.opened_at,
            half_open_at: None,
            updated_at: Utc::now(),
        }))
    }

    async fn update_circuit_breaker(
        &self,
        key: &str,
        state: crate::reliability::CircuitState,
        failure_count: u32,
        success_count: u32,
    ) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        let breaker = breakers.get_mut(key);

        match breaker {
            Some(b) => {
                let opened_at = if state == crate::reliability::CircuitState::Open
                    && b.state != crate::reliability::CircuitState::Open
                {
                    Some(Utc::now())
                } else if state == crate::reliability::CircuitState::Closed {
                    None
                } else {
                    b.opened_at
                };

                b.state = state;
                b.failure_count = failure_count;
                b.success_count = success_count;
                b.opened_at = opened_at;
            }
            None => {
                // Create if doesn't exist
                breakers.insert(
                    key.to_string(),
                    CircuitBreakerMemState {
                        state,
                        failure_count,
                        success_count,
                        opened_at: if state == crate::reliability::CircuitState::Open {
                            Some(Utc::now())
                        } else {
                            None
                        },
                    },
                );
            }
        }
        Ok(())
    }

    async fn list_circuit_breakers(&self) -> Result<Vec<CircuitBreakerState>, StoreError> {
        let breakers = self.circuit_breakers.read();
        Ok(breakers
            .iter()
            .map(|(k, b)| CircuitBreakerState {
                key: k.clone(),
                state: b.state,
                failure_count: b.failure_count,
                success_count: b.success_count,
                last_failure_at: None,
                opened_at: b.opened_at,
                half_open_at: None,
                updated_at: Utc::now(),
            })
            .collect())
    }

    async fn force_open_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        breakers.insert(
            key.to_string(),
            CircuitBreakerMemState {
                state: crate::reliability::CircuitState::Open,
                failure_count: 0,
                success_count: 0,
                opened_at: Some(Utc::now()),
            },
        );
        Ok(())
    }

    async fn force_close_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        if let Some(b) = breakers.get_mut(key) {
            b.state = crate::reliability::CircuitState::Closed;
            b.failure_count = 0;
            b.success_count = 0;
            b.opened_at = None;
            Ok(())
        } else {
            Err(StoreError::CircuitBreakerNotFound(key.to_string()))
        }
    }

    async fn delete_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        if breakers.remove(key).is_some() {
            Ok(())
        } else {
            Err(StoreError::CircuitBreakerNotFound(key.to_string()))
        }
    }
}
