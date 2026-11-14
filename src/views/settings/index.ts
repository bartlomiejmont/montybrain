// „Ustawienia" view; imported once from src/views/index.ts.

import { registerView } from "../../router/registry";
import { SettingsView } from "./SettingsView";
import "./settings.css";

export const SETTINGS_HREF = "#/ustawienia";

registerView({ id: "settings", pattern: "ustawienia", component: SettingsView });
