import { lstatSync } from "node:fs";
import { spawnSync } from "node:child_process";

function hasSymlinkedNodeModules() {
  try {
    return lstatSync(new URL("../node_modules", import.meta.url)).isSymbolicLink();
  } catch {
    return false;
  }
}

const args = ["build"];
if (hasSymlinkedNodeModules()) {
  args.push("--webpack");
}

const result = spawnSync("./node_modules/.bin/next", args, {
  cwd: new URL("..", import.meta.url),
  stdio: "inherit",
  shell: true,
});

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
