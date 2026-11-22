import { expect, test } from "bun:test";
import { authModeFromEnv, claudeEnv, NO_TOKEN, providerFromEnv } from "./auth";

const env = {
  PATH: "/usr/bin",
  ANTHROPIC_API_KEY: "sk-ant-test",
  ANTHROPIC_AUTH_TOKEN: "tok",
  ANTHROPIC_FOUNDRY_API_KEY: "foundry",
  CLAUDE_CODE_OAUTH_TOKEN: "oauth",
};

test("subscription is the default mode", () => {
  expect(authModeFromEnv({})).toBe("subscription");
  expect(authModeFromEnv({ WINGMAN_AUTH: "api-key" })).toBe("api_key");
  expect(() => authModeFromEnv({ WINGMAN_AUTH: "both" })).toThrow();
});

test("subscription mode removes API credentials and keeps the login", () => {
  const out = claudeEnv("subscription", env);
  expect(out.ANTHROPIC_API_KEY).toBeUndefined();
  expect(out.ANTHROPIC_AUTH_TOKEN).toBeUndefined();
  expect(out.ANTHROPIC_FOUNDRY_API_KEY).toBeUndefined();
  expect(out.CLAUDE_CODE_OAUTH_TOKEN).toBe("oauth");
  expect(out.PATH).toBe("/usr/bin");
});

test("api_key mode keeps the key and removes the subscription token", () => {
  const out = claudeEnv("api_key", env);
  expect(out.ANTHROPIC_API_KEY).toBe("sk-ant-test");
  expect(out.CLAUDE_CODE_OAUTH_TOKEN).toBeUndefined();
});

test("api_key mode needs a key", () => {
  expect(() => claudeEnv("api_key", { PATH: "/usr/bin" })).toThrow(/ANTHROPIC_API_KEY/);
});

test("the caller's environment is not modified", () => {
  const copy = { ...env };
  claudeEnv("subscription", copy);
  expect(copy).toEqual(env);
});

const providerEnv = {
  ...env,
  WINGMAN_AUTH: "provider",
  WINGMAN_PROVIDER_URL: "https://openrouter.ai/api/",
  WINGMAN_PROVIDER_NAME: "OpenRouter",
  WINGMAN_PROVIDER_TOKEN: "sk-or-test",
  WINGMAN_MODEL: "moonshotai/kimi-k2",
  CLAUDE_CODE_USE_BEDROCK: "1",
};

test("provider mode reads the endpoint and model", () => {
  expect(authModeFromEnv(providerEnv)).toBe("provider");
  expect(providerFromEnv(providerEnv)).toEqual({
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api",
    token: "sk-or-test",
    model: "moonshotai/kimi-k2",
    fastModel: undefined,
    webSearch: true,
  });
  const local = providerFromEnv({
    WINGMAN_PROVIDER_URL: "http://localhost:11434",
    WINGMAN_MODEL: "qwen3",
    WINGMAN_WEB_SEARCH: "0",
  });
  expect(local.name).toBe("localhost:11434");
  expect(local.token).toBeUndefined();
  expect(local.webSearch).toBe(false);
});

test("provider mode needs a URL and a model", () => {
  expect(() => providerFromEnv({ WINGMAN_MODEL: "m" })).toThrow(/WINGMAN_PROVIDER_URL/);
  expect(() => providerFromEnv({ WINGMAN_PROVIDER_URL: "not a url", WINGMAN_MODEL: "m" })).toThrow(/isn't a URL/);
  expect(() => providerFromEnv({ WINGMAN_PROVIDER_URL: "file:///etc", WINGMAN_MODEL: "m" })).toThrow(/http/);
  expect(() => providerFromEnv({ WINGMAN_PROVIDER_URL: "http://localhost:1234" })).toThrow(/WINGMAN_MODEL/);
});

test("provider mode points Claude Code at the provider, with none of the Claude credentials", () => {
  const out = claudeEnv("provider", providerEnv, providerFromEnv(providerEnv));
  expect(out.ANTHROPIC_BASE_URL).toBe("https://openrouter.ai/api");
  expect(out.ANTHROPIC_AUTH_TOKEN).toBe("sk-or-test");
  // Present and empty, so Claude Code doesn't go back to the Claude login.
  expect(out.ANTHROPIC_API_KEY).toBe("");
  expect(out.CLAUDE_CODE_OAUTH_TOKEN).toBeUndefined();
  expect(out.ANTHROPIC_FOUNDRY_API_KEY).toBeUndefined();
  expect(out.CLAUDE_CODE_USE_BEDROCK).toBeUndefined();
  expect(out.WINGMAN_PROVIDER_TOKEN).toBeUndefined();
  expect(out.ANTHROPIC_MODEL).toBe("moonshotai/kimi-k2");
  expect(out.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("moonshotai/kimi-k2");
  expect(out.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("moonshotai/kimi-k2");
  expect(out.PATH).toBe("/usr/bin");
});

test("a provider without a token still never gets the Claude login", () => {
  const local = {
    WINGMAN_PROVIDER_URL: "http://localhost:11434",
    WINGMAN_MODEL: "qwen3",
    WINGMAN_FAST_MODEL: "qwen3:4b",
  };
  const out = claudeEnv("provider", { ...env, ...local }, providerFromEnv(local));
  expect(out.ANTHROPIC_AUTH_TOKEN).toBe(NO_TOKEN);
  expect(out.CLAUDE_CODE_OAUTH_TOKEN).toBeUndefined();
  expect(out.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("qwen3:4b");
});
