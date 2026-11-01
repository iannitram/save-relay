// The Save Relay Game Mode panel (SPEC.md M15): Decky Loader shows it in the Quick Access
// menu. Placeholder: it renders the panel's views over the backend bridge; the views are
// built by their issues.

import { callable, definePlugin } from "@decky/api";
import { PanelSection, staticClasses } from "@decky/ui";
import { App } from "panel/App";
import type { ApiArgs, ApiCommand } from "panel/bridge";
import { deckyBridge } from "./bridge.ts";

const run = callable<[command: ApiCommand, args: ApiArgs], string>("run");
const bridge = deckyBridge(run);

export default definePlugin(() => ({
  name: "Save Relay",
  titleView: <div className={staticClasses.Title}>Save Relay</div>,
  content: (
    <PanelSection>
      <App bridge={bridge} />
    </PanelSection>
  ),
  icon: <span>SR</span>,
}));
