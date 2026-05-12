import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { Tool, ToolContext, ToolResult, checkPermission } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

const exec = promisify(execFile);

export class AppTool extends Tool {
  name = "app";
  description = "Open, close, focus, or list applications.";

  schema = {
    type: "object",
    required: ["action"],
    properties: {
      action: { type: "string", enum: ["open", "close", "focus", "list"] },
      name: { type: "string", description: "Application name or path — required for open/close/focus" },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    const { action, name } = params as { action: string; name?: string };

    await checkPermission(ctx, "app", `${action} ${name ?? ""}`, `App ${action}`);

    const sandbox = getSandbox(ctx.sessionId);
    if (name && !sandbox.isAppAllowed(name)) {
      return this.err("App denied", `'${name}' is not in the allowed apps list.`);
    }

    try {
      const platform = process.platform;

      switch (action) {
        case "open": {
          if (!name) return this.err("App error", "name is required for action=open");
          if (platform === "darwin") await exec("open", ["-a", name]);
          else if (platform === "win32") await exec("cmd", ["/c", "start", "", name]);
          else await exec("xdg-open", [name]);
          sandbox.recordAction("app_open", params, "ok");
          return this.ok("App open", `Opened '${name}'.`);
        }
        case "close": {
          if (!name) return this.err("App error", "name is required for action=close");
          if (platform === "darwin") {
            await exec("osascript", ["-e", `quit app "${name}"`]);
          } else if (platform === "win32") {
            await exec("taskkill", ["/IM", `${name}.exe`, "/F"]);
          } else {
            await exec("pkill", ["-f", name]);
          }
          sandbox.recordAction("app_close", params, "ok");
          return this.ok("App close", `Closed '${name}'.`);
        }
        case "focus": {
          if (!name) return this.err("App error", "name is required for action=focus");
          if (platform === "darwin") {
            await exec("osascript", ["-e", `tell application "${name}" to activate`]);
          } else if (platform === "win32") {
            await exec("powershell", [
              "-Command",
              `(New-Object -ComObject WScript.Shell).AppActivate('${name}')`,
            ]);
          } else {
            await exec("wmctrl", ["-a", name]);
          }
          sandbox.recordAction("app_focus", params, "ok");
          return this.ok("App focus", `Focused '${name}'.`);
        }
        case "list": {
          let output = "";
          if (platform === "darwin") {
            const { stdout } = await exec("osascript", [
              "-e",
              "tell application \"System Events\" to get name of every process whose background only is false",
            ]);
            output = stdout.trim();
          } else if (platform === "win32") {
            const { stdout } = await exec("tasklist", ["/fo", "csv", "/nh"]);
            output = stdout.trim();
          } else {
            const { stdout } = await exec("wmctrl", ["-l"]);
            output = stdout.trim();
          }
          sandbox.recordAction("app_list", params, "ok");
          return this.ok("App list", output);
        }
        default:
          return this.err("App error", `Unknown action: ${action}`);
      }
    } catch (e) {
      return this.err("App error", `${e}`);
    }
  }
}
