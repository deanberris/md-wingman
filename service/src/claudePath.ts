import { accessSync, constants } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";

/**
 * Works out which Claude Code executable the Agent SDK should run.
 *
 * - WINGMAN_CLAUDE_PATH, if set, always wins.
 * - Run from source (`bun run src/main.ts`), the SDK finds the copy it installed
 *   in node_modules, so we return undefined and let it.
 * - Compiled with `bun build --compile`, the SDK cannot see node_modules. We
 *   look for the user's own Claude Code install instead: on PATH, then in the
 *   usual install locations, since an app started from Finder doesn't get the
 *   shell's PATH.
 */
export function resolveClaudePath(env: NodeJS.ProcessEnv = process.env): string | undefined {
  if (env.WINGMAN_CLAUDE_PATH) return env.WINGMAN_CLAUDE_PATH;
  if (!isCompiled()) return undefined;

  const home = homedir();
  const candidates = [
    ...(env.PATH ?? "").split(delimiter).filter(Boolean).map((dir) => join(dir, "claude")),
    join(home, ".local/bin/claude"),
    join(home, ".claude/local/claude"),
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
  ];
  const found = candidates.find(isExecutable);
  if (!found) {
    throw new Error(
      "Claude Code was not found. Install it (https://claude.com/claude-code) and sign in with `claude`, or set WINGMAN_CLAUDE_PATH.",
    );
  }
  return found;
}

function isCompiled(): boolean {
  return Bun.main.startsWith("/$bunfs/") || Bun.main.startsWith("B:\\~BUN\\");
}

function isExecutable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
