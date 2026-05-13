#!/usr/bin/env node
import { main } from "../dist/cli.js";

main().catch((err) => {
  process.stderr.write(`ERROR: ${err?.message ?? String(err)}\n`);
  process.exit(1);
});
