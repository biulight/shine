#!/usr/bin/env bun

import { runProfileArtifact } from "./profile-artifact";

try {
  await runProfileArtifact("unbuild");
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
