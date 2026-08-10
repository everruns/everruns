//! Demo and test fixture capabilities.
//!
//! These capabilities exist for deterministic tests, examples, and demos
//! only. They are intentionally not part of any production capability
//! registry: product compositions (server/worker) must not register them,
//! and an architecture guard enforces that their modules are never imported
//! from production trees. Tests and examples register them explicitly, e.g.
//! `registry.register(TestMathCapability)`.

mod fake_aws;
mod fake_crm;
mod fake_financial;
mod fake_warehouse;
mod noop;
mod sample_data;
mod test_math;
mod test_weather;

pub use fake_aws::{
    AwsCreateEc2InstanceTool, AwsCreateIamUserTool, AwsCreateRdsDatabaseTool,
    AwsCreateS3BucketTool, AwsGetCloudWatchMetricsTool, AwsListEc2InstancesTool,
    AwsListIamUsersTool, AwsListRdsDatabasesTool, AwsListS3BucketsTool, AwsListSecurityGroupsTool,
    AwsStopEc2InstanceTool, FAKE_AWS_CAPABILITY_ID, FakeAwsCapability,
};
pub use fake_crm::{
    CrmAddInteractionTool, CrmCreateCustomerTool, CrmCreateTicketTool, CrmGetCustomerTool,
    CrmListCustomersTool, CrmListTicketsTool, CrmSearchCustomersTool, CrmUpdateTicketTool,
    FAKE_CRM_CAPABILITY_ID, FakeCrmCapability,
};
pub use fake_financial::{
    FAKE_FINANCIAL_CAPABILITY_ID, FakeFinancialCapability, FinanceCreateBudgetTool,
    FinanceCreateTransactionTool, FinanceForecastCashFlowTool, FinanceGetBalanceTool,
    FinanceGetExpenseReportTool, FinanceGetRevenueReportTool, FinanceListBudgetsTool,
    FinanceListTransactionsTool,
};
pub use fake_warehouse::{
    FAKE_WAREHOUSE_CAPABILITY_ID, FakeWarehouseCapability, WarehouseCreateInvoiceTool,
    WarehouseCreateOrderTool, WarehouseCreateShipmentTool, WarehouseGetInventoryTool,
    WarehouseInventoryReportTool, WarehouseListOrdersTool, WarehouseListShipmentsTool,
    WarehouseProcessReturnTool, WarehouseUpdateInventoryTool, WarehouseUpdateShipmentStatusTool,
};
pub use noop::{NOOP_CAPABILITY_ID, NoopCapability};
pub use sample_data::{SAMPLE_DATA_CAPABILITY_ID, SampleDataCapability};
pub use test_math::{
    AddTool, DivideTool, MultiplyTool, SubtractTool, TEST_MATH_CAPABILITY_ID, TestMathCapability,
};
pub use test_weather::{
    GetForecastTool, GetWeatherTool, TEST_WEATHER_CAPABILITY_ID, TestWeatherCapability,
};
