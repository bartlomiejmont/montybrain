// Built-in views. A new view (kanban, map, …) is one more `registerView` call
// here plus its component; the shell, sidebar and other views stay untouched.

import { registerView } from "../router/registry";
import { BoardView } from "./BoardView";
import { InboxView } from "./InboxView";
import { ProjectView } from "./ProjectView";
import { TodayView } from "./TodayView";

registerView({ id: "today", pattern: "dzis", component: TodayView });
registerView({ id: "inbox", pattern: "skrzynka", component: InboxView });
registerView({ id: "board", pattern: "tablica", component: BoardView });
registerView({ id: "project", pattern: "p/:project", component: ProjectView });
import "./settings"; // „Ustawienia" (registers itself)
import "./actions"; // „Akcje" (registers itself)
