import { expect, test } from "bun:test";
import { authModeFromEnv, claudeEnv } from "./auth";

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
