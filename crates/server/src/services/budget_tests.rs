// Budget system tests
//
// Tests for BudgetService (rules engine, metering, cost computation),
// storage layer (CRUD, ledger, hierarchy), and end-to-end integration.

#[cfg(test)]
mod tests {
    use crate::services::budget::BudgetService;
    use crate::storage::StorageBackend;
    use crate::storage::models::*;
    use everruns_core::budget::BudgetAction;
    use std::sync::Arc;

    fn make_db() -> Arc<StorageBackend> {
        Arc::new(StorageBackend::in_memory())
    }

    fn make_service() -> (BudgetService, Arc<StorageBackend>) {
        let db = make_db();
        let svc = BudgetService::new(db.clone());
        (svc, db)
    }

    fn make_budget_row(
        limit: f64,
        balance: f64,
        soft_limit: Option<f64>,
        currency: &str,
    ) -> BudgetRow {
        BudgetRow {
            id: uuid::Uuid::now_v7(),
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_test".into(),
            currency: currency.into(),
            limit,
            soft_limit,
            balance,
            period: None,
            metadata: None,
            status: "active".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // ========================================================================
    // Rules engine tests
    // ========================================================================

    #[test]
    fn test_rule_continue_when_budget_healthy() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 80.0, None, "usd");
        let action = svc.evaluate_rules(&budget);
        assert_eq!(action, BudgetAction::Continue);
    }

    #[test]
    fn test_rule_warn_at_80_percent_consumed() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 15.0, None, "usd");
        let action = svc.evaluate_rules(&budget);
        assert!(matches!(action, BudgetAction::Warn { .. }));
    }

    #[test]
    fn test_rule_warn_at_exactly_20_percent_remaining() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 20.0, None, "usd");
        let action = svc.evaluate_rules(&budget);
        assert!(matches!(action, BudgetAction::Warn { .. }));
    }

    #[test]
    fn test_rule_no_warn_above_20_percent() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 21.0, None, "usd");
        let action = svc.evaluate_rules(&budget);
        assert_eq!(action, BudgetAction::Continue);
    }

    #[test]
    fn test_rule_stop_when_balance_zero() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 0.0, None, "usd");
        let action = svc.evaluate_rules(&budget);
        assert!(matches!(action, BudgetAction::Stop { .. }));
    }

    #[test]
    fn test_rule_stop_when_balance_negative() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, -5.0, None, "usd");
        let action = svc.evaluate_rules(&budget);
        assert!(matches!(action, BudgetAction::Stop { .. }));
    }

    #[test]
    fn test_rule_pause_at_soft_limit() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 15.0, Some(80.0), "usd");
        let action = svc.evaluate_rules(&budget);
        assert!(matches!(action, BudgetAction::Pause { .. }));
    }

    #[test]
    fn test_rule_no_pause_before_soft_limit() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 25.0, Some(80.0), "usd");
        let action = svc.evaluate_rules(&budget);
        assert_eq!(action, BudgetAction::Continue);
    }

    #[test]
    fn test_rule_stop_takes_priority_over_pause() {
        let (svc, _) = make_service();
        let budget = make_budget_row(100.0, 0.0, Some(80.0), "usd");
        let action = svc.evaluate_rules(&budget);
        assert!(matches!(action, BudgetAction::Stop { .. }));
    }

    #[test]
    fn test_rule_stop_message_includes_currency() {
        let (svc, _) = make_service();
        let budget = make_budget_row(50.0, -2.0, None, "tokens");
        let action = svc.evaluate_rules(&budget);
        if let BudgetAction::Stop { message } = action {
            assert!(message.contains("tokens"));
        } else {
            panic!("Expected Stop action");
        }
    }

    // ========================================================================
    // Metering / cost computation tests
    // ========================================================================

    #[test]
    fn test_compute_debit_tokens_currency() {
        let (svc, _) = make_service();
        let debit = svc.compute_debit("tokens", 1500, 1000, 500, Some("gpt-4o"), Some("openai"));
        assert_eq!(debit, 1500.0);
    }

    #[test]
    fn test_compute_debit_usd_with_known_model() {
        let (svc, _) = make_service();
        let debit = svc.compute_debit(
            "usd",
            1_000_000 + 500_000,
            1_000_000,
            500_000,
            Some("gpt-4o"),
            Some("openai"),
        );
        assert!(debit > 0.0, "Debit should be positive for known model");
        assert!(debit < 100.0, "Debit should be reasonable for 1.5M tokens");
    }

    #[test]
    fn test_compute_debit_usd_with_unknown_model_falls_back_to_tokens() {
        let (svc, _) = make_service();
        let debit = svc.compute_debit(
            "usd",
            1500,
            1000,
            500,
            Some("unknown-model"),
            Some("openai"),
        );
        assert_eq!(debit, 1500.0);
    }

    #[test]
    fn test_compute_debit_credits_currency() {
        let (svc, _) = make_service();
        let debit = svc.compute_debit("credits", 5000, 3000, 2000, None, None);
        assert_eq!(debit, 5.0);
    }

    #[test]
    fn test_compute_debit_custom_currency_uses_token_count() {
        let (svc, _) = make_service();
        let debit = svc.compute_debit("my_custom", 2000, 1500, 500, None, None);
        assert_eq!(debit, 2000.0);
    }

    #[test]
    fn test_compute_debit_zero_tokens() {
        let (svc, _) = make_service();
        let debit = svc.compute_debit("tokens", 0, 0, 0, None, None);
        assert_eq!(debit, 0.0);
    }

    // ========================================================================
    // Storage layer tests (in-memory)
    // ========================================================================

    #[tokio::test]
    async fn test_create_and_get_budget() {
        let db = make_db();
        let input = CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_abc".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: Some(8.0),
            period: None,
            metadata: None,
        };
        let created = db.create_budget(input).await.unwrap();
        assert_eq!(created.limit, 10.0);
        assert_eq!(created.balance, 10.0);
        assert_eq!(created.soft_limit, Some(8.0));
        assert_eq!(created.currency, "usd");
        assert_eq!(created.status, "active");

        let fetched = db.get_budget(1, created.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.limit, 10.0);
    }

    #[tokio::test]
    async fn test_get_budget_wrong_org_returns_none() {
        let db = make_db();
        let input = CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        };
        let created = db.create_budget(input).await.unwrap();
        assert!(db.get_budget(999, created.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_list_budgets_filters() {
        let db = make_db();
        for (stype, sid) in [("session", "s1"), ("agent", "a1")] {
            db.create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: stype.into(),
                subject_id: sid.into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        }
        let all = db.list_budgets(1, None, None).await.unwrap();
        assert_eq!(all.len(), 2);
        let sessions = db.list_budgets(1, Some("session"), None).await.unwrap();
        assert_eq!(sessions.len(), 1);
        let specific = db.list_budgets(1, None, Some("a1")).await.unwrap();
        assert_eq!(specific.len(), 1);
    }

    #[tokio::test]
    async fn test_update_budget_increases_balance() {
        let db = make_db();
        let created = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        let updated = db
            .update_budget(
                1,
                created.id,
                UpdateBudgetRow {
                    limit: Some(20.0),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.limit, 20.0);
        assert_eq!(updated.balance, 20.0);
    }

    #[tokio::test]
    async fn test_delete_budget_soft_deletes() {
        let db = make_db();
        let created = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        assert!(db.delete_budget(1, created.id).await.unwrap());
        let fetched = db.get_budget(1, created.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, "disabled");
        assert!(db.list_budgets(1, None, None).await.unwrap().is_empty());
    }

    // ========================================================================
    // Ledger tests
    // ========================================================================

    #[tokio::test]
    async fn test_ledger_entry_reduces_balance() {
        let db = make_db();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        let (entry, updated) = db
            .create_budget_ledger_entry(CreateBudgetLedgerRow {
                budget_id: budget.id,
                amount: 3.0,
                meter_source: "llm_tokens".into(),
                ref_type: Some("llm_generation".into()),
                ref_id: Some(uuid::Uuid::now_v7()),
                session_id: None,
                description: Some("test debit".into()),
            })
            .await
            .unwrap();
        assert_eq!(entry.amount, 3.0);
        assert_eq!(updated.balance, 7.0);
    }

    #[tokio::test]
    async fn test_ledger_multiple_debits_accumulate() {
        let db = make_db();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "tokens".into(),
                limit: 1000.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        for _ in 0..5 {
            db.create_budget_ledger_entry(CreateBudgetLedgerRow {
                budget_id: budget.id,
                amount: 100.0,
                meter_source: "llm_tokens".into(),
                ref_type: None,
                ref_id: None,
                session_id: None,
                description: None,
            })
            .await
            .unwrap();
        }
        let fetched = db.get_budget(1, budget.id).await.unwrap().unwrap();
        assert_eq!(fetched.balance, 500.0);
    }

    #[tokio::test]
    async fn test_ledger_negative_amount_is_credit() {
        let db = make_db();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        db.create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: 8.0,
            meter_source: "llm_tokens".into(),
            ref_type: None,
            ref_id: None,
            session_id: None,
            description: None,
        })
        .await
        .unwrap();
        let (_, updated) = db
            .create_budget_ledger_entry(CreateBudgetLedgerRow {
                budget_id: budget.id,
                amount: -5.0,
                meter_source: "manual".into(),
                ref_type: Some("top_up".into()),
                ref_id: None,
                session_id: None,
                description: Some("top up".into()),
            })
            .await
            .unwrap();
        assert_eq!(updated.balance, 7.0);
    }

    #[tokio::test]
    async fn test_list_ledger_pagination() {
        let db = make_db();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "tokens".into(),
                limit: 10000.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        for i in 0..10 {
            db.create_budget_ledger_entry(CreateBudgetLedgerRow {
                budget_id: budget.id,
                amount: (i + 1) as f64,
                meter_source: "llm_tokens".into(),
                ref_type: None,
                ref_id: None,
                session_id: None,
                description: Some(format!("entry {i}")),
            })
            .await
            .unwrap();
        }
        assert_eq!(
            db.list_budget_ledger(budget.id, 3, 0).await.unwrap().len(),
            3
        );
        assert_eq!(
            db.list_budget_ledger(budget.id, 3, 3).await.unwrap().len(),
            3
        );
        assert_eq!(
            db.list_budget_ledger(budget.id, 100, 0)
                .await
                .unwrap()
                .len(),
            10
        );
    }

    // ========================================================================
    // Budget hierarchy tests
    // ========================================================================

    #[tokio::test]
    async fn test_active_budgets_for_session_hierarchy() {
        let db = make_db();
        db.create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
        db.create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "agent".into(),
            subject_id: "agent_1".into(),
            currency: "usd".into(),
            limit: 50.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
        db.create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_other".into(),
            currency: "usd".into(),
            limit: 999.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

        let budgets = db
            .get_active_budgets_for_session(1, "session_1", Some("agent_1"), None, None)
            .await
            .unwrap();
        assert_eq!(budgets.len(), 2);
        let subjects: Vec<&str> = budgets.iter().map(|b| b.subject_id.as_str()).collect();
        assert!(subjects.contains(&"session_1"));
        assert!(subjects.contains(&"agent_1"));
        assert!(!subjects.contains(&"session_other"));
    }

    #[tokio::test]
    async fn test_active_budgets_excludes_disabled() {
        let db = make_db();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "session_1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        db.delete_budget(1, budget.id).await.unwrap();
        assert!(
            db.get_active_budgets_for_session(1, "session_1", None, None, None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ========================================================================
    // End-to-end: check_budgets_for_session
    // ========================================================================

    #[tokio::test]
    async fn test_check_budgets_no_budgets_returns_ok() {
        let (svc, _) = make_service();
        let result = svc.check_budgets_for_session(1, "session_1", None).await;
        assert_eq!(result.action, "continue");
    }

    #[tokio::test]
    async fn test_check_budgets_healthy_budget_returns_continue() {
        let (svc, db) = make_service();
        db.create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 100.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
        let result = svc.check_budgets_for_session(1, "session_1", None).await;
        assert_eq!(result.action, "continue");
    }

    #[tokio::test]
    async fn test_check_budgets_exhausted_budget_returns_stop() {
        let (svc, db) = make_service();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "session_1".into(),
                currency: "tokens".into(),
                limit: 100.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        db.create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: 100.0,
            meter_source: "llm_tokens".into(),
            ref_type: None,
            ref_id: None,
            session_id: None,
            description: None,
        })
        .await
        .unwrap();
        db.set_budget_status(budget.id, "exhausted").await.unwrap();
        let result = svc.check_budgets_for_session(1, "session_1", None).await;
        assert!(result.should_stop());
    }

    #[tokio::test]
    async fn test_check_budgets_paused_budget_returns_pause() {
        let (svc, db) = make_service();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "session_1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        db.set_budget_status(budget.id, "paused").await.unwrap();
        let result = svc.check_budgets_for_session(1, "session_1", None).await;
        assert!(result.should_pause());
    }

    #[tokio::test]
    async fn test_check_budgets_most_restrictive_wins() {
        let (svc, db) = make_service();
        db.create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
        let agent_budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "agent".into(),
                subject_id: "agent_1".into(),
                currency: "usd".into(),
                limit: 50.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        db.create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: agent_budget.id,
            amount: 50.0,
            meter_source: "llm_tokens".into(),
            ref_type: None,
            ref_id: None,
            session_id: None,
            description: None,
        })
        .await
        .unwrap();
        db.set_budget_status(agent_budget.id, "exhausted")
            .await
            .unwrap();
        // Session budget healthy, but agent budget exhausted → should stop
        let result = svc
            .check_budgets_for_session(1, "session_1", Some("agent_1"))
            .await;
        assert_eq!(result.action, "stop");
    }

    // ========================================================================
    // Set budget status + DTO conversion tests
    // ========================================================================

    #[tokio::test]
    async fn test_set_budget_status() {
        let db = make_db();
        let budget = db
            .create_budget(CreateBudgetRow {
                org_id: 1,
                subject_type: "session".into(),
                subject_id: "s1".into(),
                currency: "usd".into(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .await
            .unwrap();
        let updated = db
            .set_budget_status(budget.id, "paused")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "paused");
        let updated2 = db
            .set_budget_status(budget.id, "active")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated2.status, "active");
    }

    #[test]
    fn test_row_to_budget_dto() {
        let row = make_budget_row(100.0, 75.0, Some(80.0), "usd");
        let dto = BudgetService::row_to_budget(&row);
        assert_eq!(dto.limit, 100.0);
        assert_eq!(dto.balance, 75.0);
        assert_eq!(dto.soft_limit, Some(80.0));
        assert_eq!(dto.currency, "usd");
        assert!(matches!(
            dto.status,
            everruns_core::budget::BudgetStatus::Active
        ));
    }

    #[test]
    fn test_row_to_ledger_entry_dto() {
        let row = BudgetLedgerRow {
            id: uuid::Uuid::now_v7(),
            budget_id: uuid::Uuid::now_v7(),
            amount: 5.5,
            meter_source: "llm_tokens".into(),
            ref_type: Some("llm_generation".into()),
            ref_id: Some(uuid::Uuid::now_v7()),
            session_id: None,
            description: Some("test".into()),
            created_at: chrono::Utc::now(),
        };
        let dto = BudgetService::row_to_ledger_entry(&row);
        assert_eq!(dto.amount, 5.5);
        assert_eq!(dto.meter_source, "llm_tokens");
        assert_eq!(dto.description, Some("test".into()));
    }
}
