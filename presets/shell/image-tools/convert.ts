// Convert images to JPEG, PNG, or WebP.
import { runImageTool } from "./image-tools.ts";

if (import.meta.main) {
  process.exit(await runImageTool("convert"));
}
