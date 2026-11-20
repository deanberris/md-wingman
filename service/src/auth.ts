// Which credential Claude Code uses for the service's requests.
//
// - "subscription" (the default): Claude Code uses the login from `claude`
//   (or a CLAUDE_CODE_OAUTH_TOKEN from `claude setup-token`), so usage counts
//   against the user's Claude plan. API keys are removed from the environment,
//   so a key that happens to be set can't quietly switch billing to the API.
// - "api_key": Claude Code uses ANTHROPIC_API_KEY and bills the Claude API.
//   Subscription tokens are removed so the key is what gets used.
//
// Choose with WINGMAN_AUTH=subscription or WINGMAN_AUTH=api_key.

export type AuthMode = "subscription" | "api_key";

/** Variables that make Claude Code bill an API account instead of the plan. */
const API_CREDENTIAL_VARS = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_FOUNDRY_API_KEY"];

/** Variables that carry a subscription login. */
const SUBSCRIPTION_CREDENTIAL_VARS = ["CLAUDE_CODE_OAUTH_TOKEN"];

export function authModeFromEnv(env: NodeJS.ProcessEnv = process.env): AuthMode {
  const value = (env.WINGMAN_AUTH ?? "subscription").trim().toLowerCase().replace("-", "_");
  if (value === "subscription" || value === "api_key") return value;
  throw new Error(`WINGMAN_AUTH must be "subscription" or "api_key", not "${env.WINGMAN_AUTH}"`);
}

/** The environment to start Claude Code with, for the given mode. */
export function claudeEnv(mode: AuthMode, env: NodeJS.ProcessEnv = process.env): Record<string, string | undefined> {
  const out: Record<string, string | undefined> = { ...env };
  const remove = mode === "subscription" ? API_CREDENTIAL_VARS : SUBSCRIPTION_CREDENTIAL_VARS;
  for (const name of remove) delete out[name];
  if (mode === "api_key" && !out.ANTHROPIC_API_KEY) {
    throw new Error("WINGMAN_AUTH=api_key needs ANTHROPIC_API_KEY to be set");
  }
  return out;
}
