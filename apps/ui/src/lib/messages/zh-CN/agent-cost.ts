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
};
