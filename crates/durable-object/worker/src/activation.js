import { HttpError, requireValue } from "./storage.js";

// Alarms and native dispatch are infrastructure; agent planning stays in Rust.
export async function runActivation(authority, execute) {
  const j = authority.journal;
  const dispatch = await j.mutate((state) => {
    if (!state.runnable || state.suspended || state.expires > j.now() || state.retry_at > j.now())
      return null;
    // Persist another wake before external I/O. An executor crash or a lost
    // dispatch reply cannot erase the obligation to run this session.
    state.retry_at = j.now() + Math.min(60000, 2000 * 2 ** Math.min(state.failures, 5));
    return {
      fence: state.fence,
      activation: {
        tenant: state.tenant,
        session_id: state.session_id,
        binding: {agent:state.binding.agent,build_id:state.binding.build_id},
      },
    };
  });
  if (!dispatch) return;
  const activation = dispatch.activation;
  let outcome = 503;
  let failureCode = "executor_unavailable";
  try {
    outcome = await execute(activation);
    failureCode = `executor_http_${outcome}`;
  } catch (error) {
    outcome = 503;
    failureCode =
      error instanceof HttpError
        ? "invalid_executor_configuration"
        : `executor_${error?.name ?? "error"}`;
  }
  await j.mutate((state) => {
    if (state.fence !== dispatch.fence && outcome >= 400) return;
    if (outcome === 409) {
      state.suspended = true;
      state.last_error = "incompatible_executor_build";
    } else if (outcome >= 400) {
      state.failures++;
      state.last_error = failureCode;
    } else {
      state.failures = 0;
      state.last_error = null;
    }
  });
}

export async function runAlarm(authority, env, fetcher = fetch) {
  return runActivation(authority, async (activation) => {
    const url = new URL(env.EXECUTOR_URL);
    requireValue(
      url.protocol === "https:" ||
        (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)),
      503,
      "invalid executor endpoint",
    );
    requireValue(
      !url.username && !url.password && !url.search && !url.hash,
      503,
      "executor URL cannot carry credentials or query data",
    );
    requireValue(
      typeof env.EXECUTOR_TOKEN === "string" && env.EXECUTOR_TOKEN.length >= 32,
      503,
      "executor secret is not configured",
    );
    const response = await fetcher(url, {
      method: "POST",
      redirect: "manual",
      signal: AbortSignal.timeout(5000),
      headers: {
        authorization: `Bearer ${env.EXECUTOR_TOKEN}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(activation),
    });
    const outcome = response.status;
    await response.body?.cancel();
    requireValue(outcome < 300 || outcome >= 400, 503, "executor redirect rejected");
    return outcome;
  });
}
