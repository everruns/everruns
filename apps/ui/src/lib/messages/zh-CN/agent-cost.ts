export const messages: Readonly<Record<string, string>> = {
  "/ task": "/ 任务",
  "Advanced assumptions": "高级假设",
  "Agent model": "智能体模型",
  "Allow for variation": "考虑波动",
  "Answer a question or write a short reply": "回答问题或撰写简短回复",
  "Calculating your estimate…": "正在计算预估费用…",
  "Check the numbers under Advanced assumptions, or choose a task to reset them.":
    "请检查“高级假设”中的数值，或选择任务以重置。",
  "Choose a model for this agent before estimating its cost.":
    "请先为此智能体选择模型，再预估费用。",
  "Choose the kind of work and how often you’ll use this agent.":
    "选择工作类型以及使用此智能体的频率。",
  "Cost breakdown": "费用明细",
  "Deep research": "深入研究",
  "Enter a whole number from 0 to 1,000,000.": "请输入 0 到 1,000,000 之间的整数。",
  "Enter a whole number from {min} to {max}.": "请输入 {min} 到 {max} 之间的整数。",
  "Estimate cost": "预估费用",
  "Estimated AI cost · USD": "预估 AI 费用 · USD",
  "Explore a topic and compare several sources": "探索主题并比较多个来源",
  "Find information and put together an answer": "查找信息并整理答案",
  "For simpler or more involved tasks": "适用于较简单或较复杂的任务",
  "Harness model": "运行框架模型",
  "Include follow-up calls after tools.": "包含使用工具后的后续调用。",
  "Include reasoning and tool-call arguments.": "包含推理和工具调用参数。",
  "Input · {count} tokens": "输入 · {count} 个 token",
  "Larger tasks may need a different model.": "较大的任务可能需要其他模型。",
  "Long-context pricing applies to one or more calls.": "一个或多个调用适用长上下文定价。",
  "Model calls per task": "每项任务的模型调用次数",
  "Monthly cost": "每月费用",
  "Monthly cost assumes 30 days of use. Each task starts a new conversation.":
    "每月费用按使用 30 天计算。每项任务开始一个新对话。",
  "Multi-step task": "多步骤任务",
  "New tool results added before the next call.": "下次调用前添加的新工具结果。",
  "One task is one request, such as “research this company.”":
    "一项任务对应一个请求，例如“研究这家公司”。",
  "Output is capped at the configured agent or model limit in affected scenarios.":
    "在相关情形中，输出受智能体或模型配置上限的限制。",
  "Output tokens per call": "每次调用的输出 token 数",
  "Output · {count} tokens": "输出 · {count} 个 token",
  "Quick answer": "快速回答",
  Retry: "重试",
  "Starting input tokens": "初始输入 token 数",
  "Task assumptions": "任务假设",
  "Tasks per day": "每天的任务数",
  "The task choices fill in these assumptions for you. Adjust them if you know how much work your agent usually does.":
    "任务选项会自动填写这些假设。如果你了解智能体通常的工作量，可以调整。",
  "This task is too large for the selected model. Try a smaller task or adjust Advanced assumptions.":
    "此任务超出了所选模型的处理范围。请尝试较小的任务或调整“高级假设”。",
  "Tool result tokens per call": "每次调用的工具结果 token 数",
  "Using {model}": "使用 {model}",
  "We can’t estimate this setup yet. Check the model and the agent’s output limit.":
    "暂时无法预估此配置。请检查模型和智能体的输出上限。",
  "We couldn’t load your estimate. Please try again.": "无法加载预估费用。请重试。",
  "We don’t have prices for this model yet. Try another model to see an estimate.":
    "暂无此模型的价格。请尝试其他模型以查看预估费用。",
  "What does a task look like?": "任务是什么类型？",
  "What will this agent cost?": "这个智能体要花多少钱？",
  "Where the cost comes from": "费用来源",
  "Your message and any starting context.": "你的消息及初始上下文。",
  "{count} tasks per month": "每月 {count} 项任务",
  "{range} for lighter to heavier tasks": "简单到复杂任务的费用为 {range}",
  "Enter tasks per day below": "请在下方输入每天的任务数",
  "A guide, not a fixed price or spending limit. Includes AI usage only. Paid tools, hosting and other service fees are extra. Trying this calculator is free and won’t run or change your agent.":
    "这只是参考，并非固定价格或支出上限。仅包含 AI 使用费用。付费工具、托管和其他服务费用另计。使用此计算器免费，且不会运行或更改智能体。",
  "Includes roughly {count} tokens of instructions, tools and message overhead on every call. Previous outputs and tool results accumulate in the conversation.":
    "每次调用包含约 {count} 个 token 的指令、工具和消息开销。之前的输出和工具结果会在对话中累积。",
  "Planning estimate, not a quote or spending limit. Lighter and heavier scenarios use half to twice the model calls, output and tool results. This is a sensitivity range, not a confidence interval or a maximum.":
    "这是规划预估，并非报价或支出上限。较轻和较重的情形使用一半到两倍的模型调用、输出和工具结果。这是敏感性范围，而非置信区间或最大值。",
  "Uses catalog token prices and approximate text sizing, with no cache discount. Actual costs depend on the task, retries, reasoning, cache reads/writes and provider pricing. Excludes sandbox time, paid tools, media, extra models or subagents, and platform fees.":
    "采用目录中的 token 价格和近似文本大小，不计缓存折扣。实际费用取决于任务、重试、推理、缓存读写和供应商定价。不包含沙盒时间、付费工具、媒体、额外模型或子智能体以及平台费用。",
  "Uses the current agent draft. Estimator inputs do not change the agent. Run representative tasks in Playground and compare their recorded usage before setting a budget.":
    "使用当前智能体草稿。估算器输入不会更改智能体。设置预算前，请在 Playground 中运行代表性任务并比较记录的用量。",
  "Smart estimate": "智能估算",
  "Manual calculator": "手动计算器",
  "What should we plan for? (optional)": "要规划什么？（可选）",
  "Infer typical tasks from this agent": "根据此智能体推断典型任务",
  "Update estimate": "更新估算",
  "AI suggests tasks from your agent. Usage sliders update instantly without another AI call.":
    "AI 根据智能体建议任务。调整用量滑块会立即更新预测，无需再次调用 AI。",
  "Understanding your agent and estimating typical tasks…": "正在分析智能体并估算典型任务…",
  "Smart estimates are unavailable right now. Retry or use the manual calculator.":
    "智能估算暂不可用。请重试或使用手动计算器。",
  "Cost projection": "费用预测",
  "Average per task": "每项任务平均费用",
  "Daily cost": "每日费用",
  "30-day month · Each task starts a new conversation": "按每月 30 天计算 · 每项任务开始一个新对话",
  "Check task assumptions and select at least one task. Oversized tasks cannot be priced.":
    "请检查任务假设并选择至少一项任务。超出限制的任务无法估价。",
  "Usage projection": "用量预测",
  "By tasks": "按任务",
  "By users": "按用户",
  "Active users": "活跃用户",
  "Tasks per user per day": "每位用户每天的任务数",
  "Enter a whole number within the displayed limits.": "请输入指定范围内的整数。",
  "Task mix": "任务组合",
  "Typical tasks for your agent": "智能体的典型任务",
  "Suggested by AI, not measured usage. Equal mix to start; adjust relative frequency or set a task to zero.":
    "由 AI 建议，并非实测用量。初始均匀分配；可调整相对频率或将某项任务设为零。",
  Frequency: "频率",
  "Monthly spend as usage grows": "用量增长时的月度费用",
  "Monthly planning range: {low} – {high}": "月度规划范围：{low} – {high}",
  "Largest cost driver: {task}": "最大费用来源：{task}",
  "Edit task": "编辑任务",
  "Task example": "任务示例",
  "Changing the example does not recalculate assumptions. Use the planning input above to generate a new estimate.":
    "修改示例不会重新计算假设。请使用上方规划输入生成新估算。",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Current catalog prices and model limits apply. Cache discounts are not assumed.":
    "每次调用均计入指令和工具；先前输出及工具结果会累积。使用当前目录价格和模型限制，不假定缓存折扣。",
  "Planning scenarios, not quotes or spending limits. The shaded range varies task complexity, not statistical confidence. AI token costs only: paid tools, hosting, media, other agents and service fees are extra. No tasks are executed and your agent is not changed.":
    "这些是规划场景，并非报价或支出上限。阴影范围体现任务复杂度变化，不代表统计置信度。仅含 AI token 费用：付费工具、托管、媒体、其他智能体及服务费另计。不会执行任务或修改智能体。",
  "{label} slider": "{label}滑块",
  "Monthly cost projection with lighter and heavier task scenarios":
    "轻量和复杂任务场景的月度费用预测",
  "{count} tasks / day": "每天 {count} 项任务",
  "At current usage": "按当前用量",
  "/ month": "/ 月",
  "Sandbox runtime": "沙盒运行时间",
  "Paid tools": "付费工具",
  "Media generation": "媒体生成",
  "Subagent work": "子代理工作",
  "Hosting and platform fees": "托管和平台费用",
  "Other operating costs": "其他运行费用",
  "Cost category": "费用类别",
  "Add cost": "添加费用",
  "Paid from credits": "从额度扣除",
  "Billed externally": "由外部服务收费",
  "Quantity per month": "每月用量",
  "Quantity per task": "每项任务用量",
  "USD per {unit}": "每 {unit} 的美元价格",
  "Enter provider price": "输入服务商价格",
  "Estimated operating cost · USD": "预计运行费用 · 美元",
  "External bills": "外部账单",
  "Loading service prices…": "正在加载服务价格…",
  "Service prices are unavailable. Reopen the estimator to retry.":
    "服务价格暂不可用。请重新打开估算器重试。",
  "Prices needed for: {items}": "需要以下项目的价格：{items}",
  "Check resource quantities and prices. Use nonnegative numbers.":
    "请检查资源用量和价格，使用非负数。",
  "Largest model cost driver: {task}": "最大的模型费用来源：{task}",
  "Extra model spend for retries (%)": "模型重试额外费用（%）",
  "Cached input reads (%)": "缓存输入读取（%）",
  "Cache writes (%)": "缓存写入（%）",
  "Monthly estimate: {amount}": "每月估算：{amount}",
  "Model usage": "模型用量",
  minute: "分钟",
  call: "次调用",
  item: "项",
  task: "任务",
  month: "月",
  cpu_second: "CPU 秒",
  search: "次搜索",
  request: "次请求",
  MB: "MB",
  "Include resources used by an average task. External providers bill separately from prepaid credits.":
    "添加每项平均任务使用的资源。外部服务商的账单与预付额度分开计算。",
  "Use the full sandbox instance price, including memory. For subagents, enter the cost of one child task. Include idle time and repeated tool work in quantities; monthly fees are added once.":
    "请输入包含内存的完整沙盒实例价格。子代理请输入每项子任务的费用。用量应包含空闲时间和重复工具工作；月费只计入一次。",
  "Per-task cost excludes fixed monthly fees. Daily cost includes one thirtieth of monthly fees. Only included resources are estimated.":
    "每项任务费用不含固定月费。每日费用包含月费的三十分之一。仅估算已添加的资源。",
  "Cache shares apply to input tokens across all tasks and must total at most 100%. Retry allowance increases model spend only; repeated tools and sandbox time belong in resource quantities.":
    "缓存比例适用于所有任务的输入令牌，总和不得超过 100%。重试预留仅增加模型费用；重复工具调用和沙盒时间应计入资源用量。",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Catalog prices and model limits apply. Cache savings require explicit assumptions.":
    "每次调用均计入指令和工具定义，先前输出与工具结果逐步累积。使用目录价格和模型限制。缓存节省需要明确设置假设。",
  "Planning scenarios, not quotes or spending limits. The shaded range varies model workload; resource assumptions stay fixed. Add paid tools, sandbox time, media, subagents and fees to include them. No tasks are executed and your agent is not changed.":
    "这是规划场景，并非报价或支出限额。阴影范围表示模型工作量变化，资源假设保持不变。请添加付费工具、沙盒时间、媒体、子代理及费用。不会执行任务或更改代理。",
};
