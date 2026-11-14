// „Akcje" view; imported once from src/views/index.ts.

import { registerView } from "../../router/registry";
import { runStore } from "../../store/runs";
import { ActionsView } from "./ActionsView";
import "./actions.css";

export const ACTIONS_HREF = "#/akcje";

registerView({ id: "actions", pattern: "akcje", component: ActionsView });

// Subscribe to the backend's run events once for the whole app lifetime, so a
// run keeps streaming while another page is open.
void runStore.start().catch(() => {
  /* the view shows its own error when it loads the list */
});
