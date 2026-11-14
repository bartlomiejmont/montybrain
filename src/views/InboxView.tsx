import { TaskList } from "../components/TaskList";
import { tasksAt } from "../lib/group";
import { useView } from "../store/vault";

const INBOX = { kind: "inbox" } as const;

export function InboxView() {
  const view = useView();
  const tasks = tasksAt(view.snapshot, INBOX);
  const open = tasks.filter((t) => t.status !== "done").length;

  return (
    <div className="view">
      <header className="view-head">
        <h1 className="view-title">Skrzynka</h1>
        <p className="view-sub">Wszystko, co jeszcze nie ma miejsca. Przenieś do projektu, gdy będzie jasne, gdzie należy.</p>
      </header>
      <section className="group" aria-label="Zadania w skrzynce">
        <TaskList
          listId="inbox"
          tasks={tasks}
          addTo={INBOX}
          addPlaceholder="Dodaj do skrzynki"
          autoFocusAdd={open === 0}
          addFirst
          empty={
            <div className="empty compact">
              <p className="empty-title">Skrzynka pusta.</p>
              <p className="empty-text">Zapisz tu cokolwiek przyjdzie do głowy — porządek zrobisz później.</p>
            </div>
          }
        />
      </section>
    </div>
  );
}
