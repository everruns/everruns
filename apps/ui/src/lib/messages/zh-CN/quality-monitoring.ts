export const messages: Readonly<Record<string, string>> = {
  "Compaction quality monitoring": "上下文压缩质量监测",
  "Automatically check compaction in real sessions. Operational checks use no judge model; sampled continuity checks use the organization model selected below.":
    "自动检查真实会话中的上下文压缩。运行检查不使用评判模型；抽样连续性检查使用下方选择的组织模型。",
  "Findings stay in this organization. Sampled excerpts are processed by its selected provider. Monitoring is read-only, and findings are retained for 30 days.":
    "结果保留在此组织中。抽样片段由组织选定的提供商处理。监测仅进行读取，结果保留 30 天。",
  "Enable automatic monitoring": "启用自动监测",
  "Monitoring model": "监测模型",
  "Mechanical checks only": "仅机械检查",
  "Daily monitoring budget (USD)": "每日监测预算（USD）",
  "Continuity sample (%)": "连续性抽样比例（%）",
  "Excluded session tags": "排除的会话标签",
  "A zero budget enables mechanical checks only. The daily UTC cap includes conservative retry reservations; uncertain calls keep their allowance for that day. Disabling monitoring stops new assessments, including queued work.":
    "预算为零时仅执行机械检查。每日 UTC 上限包括保守的重试预留额度；结果不确定的调用会在当天保留其额度。禁用后将停止新的评估，包括排队任务。",
  "Monitoring coverage · 7 days": "监测覆盖情况 · 7 天",
  "Could not load monitoring coverage.": "无法加载监测覆盖情况。",
  "Today: {reserved} reserved allowance · {reported} provider-reported cost":
    "今天：预留额度 {reserved} · 提供商报告的费用 {reported}",
  "No compaction assessments yet.": "尚无上下文压缩评估。",
  "Continuity checks": "连续性检查",
  "Operational checks": "运行检查",
  "Passed {passed} · Failed {failed} · Insufficient evidence {unknown} · Pending {pending}":
    "通过 {passed} · 失败 {failed} · 证据不足 {unknown} · 待处理 {pending}",
  "Recent findings": "最近结果",
  "An operational pass confirms the reported execution step. Continuity verdicts apply to sampled evidence and do not prove complete memory preservation or causality.":
    "运行检查通过表示已确认报告的执行步骤。连续性结论仅适用于抽样证据，不能证明完整记忆保留或因果关系。",
  "Could not load monitoring findings.": "无法加载监测结果。",
  "Insufficient evidence": "证据不足",
  "Select a monitoring model before enabling paid checks.": "启用付费检查前，请选择监测模型。",
  "Compaction monitoring": "上下文压缩监测",
  "System-managed checks configured in organization settings.": "系统管理的检查在组织设置中配置。",
  "Agree with assessment": "同意评估",
  "Disagree with assessment": "不同意评估",
  "Review recorded": "已记录反馈",
  "Could not save assessment review.": "无法保存评估反馈。",
};
