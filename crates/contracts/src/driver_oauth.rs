/// Wire flavor of a driver's interactive OAuth connect flow.
///
/// Drivers declare their flow; the host owns browser navigation, encrypted
/// storage, and who may use the resulting connection.
///
/// Hosts implement the declared flow and its storage policy. ChatGPT plan
/// connections are personal and require session-scoped credential resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverOAuthFlow {
    /// OpenRouter one-click PKCE
    /// (<https://openrouter.ai/docs/guides/overview/auth/oauth>): redirect the
    /// admin to `authorize_url?callback_url=..&code_challenge=..&code_challenge_method=S256`
    /// (the host also sends `key_label` to prefill the created key's name),
    /// then POST JSON `{code, code_verifier, code_challenge_method}` to
    /// `token_url`; the `key` field of the response is the user-controlled API
    /// key to store. No client registration or secret is required (public PKCE
    /// client).
    OpenRouterPkce,
    /// Per-account dynamic registration, with a loopback callback and rotating tokens.
    ChatGptPlan,
}
