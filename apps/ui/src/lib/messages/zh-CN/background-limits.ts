// Simplified Chinese product copy for background limits (Settings → Limits, schedule limits).
export const messages: Readonly<Record<string, string>> = {
  Limits: "限制",
  "Limit what schedules and other unattended work may run and spend":
    "限制计划任务和其他无人值守工作可运行的次数和可花费的金额",
  "Unable to load limits.": "无法加载限制。",
  "Unattended work": "无人值守工作",
  "Limits for work no person started: schedules agents create, trigger runs and automatic retries. A person can still extend a schedule from its session.":
    "针对非人工启动的工作的限制：智能体创建的计划任务、触发器运行和自动重试。人员仍可在会话中延长计划任务。",
  "Longest an agent's schedule may run": "智能体计划任务的最长运行时间",
  "Up to 7 days. The agent must set an end, and it cannot ask for more than this.":
    "最多 7 天。智能体必须设置结束时间，且不能超出此值。",
  "Most runs per agent schedule": "每个智能体计划任务的最多运行次数",
  "Up to 96 runs. The schedule stops at this count or at its end time, whichever is first.":
    "最多 96 次。计划任务在达到此次数或结束时间时停止，以先到者为准。",
  "Default budget per schedule": "每个计划任务的默认预算",
  "What a schedule may spend in total when the agent does not ask for a budget.":
    "智能体未申请预算时，计划任务可花费的总额。",
  "Largest budget an agent may give a schedule": "智能体可为计划任务设置的最高预算",
  "The most an agent may ask for. A person can raise a single schedule past it.":
    "智能体可申请的上限。人员可将单个计划任务提高到超出此值。",
  "Daily cap on unattended work (schedules, triggers, retries)":
    "无人值守工作的每日上限（计划任务、触发器、重试）",
  "What all unattended work in this organization may spend per day, together.":
    "此组织中所有无人值守工作每天合计可花费的金额。",
  "No cap": "不设上限",
  "Only organization owners and admins can change these limits.":
    "只有组织所有者和管理员可以更改这些限制。",
  "Could not save limits.": "无法保存限制。",
  runs: "次",
  "Schedule lifetime must be a whole number of days from 1 to 7.":
    "计划任务时长必须是 1 到 7 之间的整数天。",
  "Runs per schedule must be a whole number from 1 to 96.":
    "每个计划任务的运行次数必须是 1 到 96 之间的整数。",
  "Default budget per schedule must be more than $0.": "每个计划任务的默认预算必须大于 $0。",
  "Largest schedule budget must be more than $0.": "计划任务最高预算必须大于 $0。",
  "Default budget per schedule cannot be more than the largest budget.":
    "每个计划任务的默认预算不能超过最高预算。",
  "Daily cap must be more than $0, or turn on No cap.": "每日上限必须大于 $0，或开启不设上限。",
  "Unattended work (daily)": "无人值守工作（每日）",
  "Budget used up": "预算已用尽",
  " · Run {value0} of {value1}": " · 第 {value0} 次，共 {value1} 次",
  " · Ends {value0}": " · 结束于 {value0}",
  " · {value0} of {value1}": " · {value0} / {value1}",
  "Stops when: {value0}": "停止条件：{value0}",
  Extend: "延长",
  "Re-enable": "重新启用",
  "Could not re-enable the schedule.": "无法重新启用计划任务。",
  "Extend schedule": "延长计划任务",
  "Give this schedule more time, runs or budget. You can go past the limits agents are held to.":
    "为此计划任务增加时间、运行次数或预算。你可以超出智能体所受的限制。",
  "Ends at": "结束时间",
  "Most runs": "最多运行次数",
  "Runs so far: {value0}": "已运行次数：{value0}",
  "Budget (USD)": "预算（USD）",
  "Could not extend the schedule.": "无法延长计划任务。",
  "The end time must be in the future.": "结束时间必须在将来。",
  "Runs must be a whole number above 0.": "运行次数必须是大于 0 的整数。",
  "The budget must be more than $0.": "预算必须大于 $0。",
  "Change at least one value to extend.": "至少更改一个值才能延长。",
};
