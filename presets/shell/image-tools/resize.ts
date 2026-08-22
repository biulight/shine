// Resize JPEG, PNG, and WebP images to fit configured bounds.
import { runImageTool } from "./image-tools.ts";

if (import.meta.main) {
  process.exit(await runImageTool("resize"));
}
