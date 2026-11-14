import { FileText, FolderGit2, X } from "lucide-react";
import { useState } from "react";

import type { Project, ProjectState, Repo } from "../api";
import { PROJECT_COLORS, PROJECT_STATES, PROJECT_STATE_LABEL } from "../lib/labels";
import { closeAfterSave, once, projectPatch, submitAction } from "../lib/form";
import { dropDrafts, useDraft, useSubmitting } from "../store/drafts";
import { useVault } from "../store/vault";
import { useActions } from "./actions";
import { Button, ConfirmButton, DraftHint, IconButton, ProjectDot, SaveActions, Segmented } from "./ui";

function ProjectEditor({ project, onDone }: { project: Project; onDone: () => void }) {
  const { run, notify } = useVault();
  // Drafts (store/drafts.ts): an untouched field follows the file, a field the
  // user changed is kept and flagged when the file changes under it, and saving
  // sends only the fields the user changed.
  const dk = `pe:${project.slug}:`;
  const nameD = useDraft(`${dk}name`, project.name);
  const colorD = useDraft(`${dk}color`, project.color ?? "");
  const stateD = useDraft(`${dk}state`, project.state);
  const descD = useDraft(`${dk}description`, project.description);
  const reposD = useDraft(`${dk}repos`, JSON.stringify(project.repos));
  const name = nameD.draft;
  const color = colorD.draft;
  const state = stateD.draft as ProjectState;
  const description = descD.draft;
  const repos = JSON.parse(reposD.draft) as Repo[];
  const setRepos = (r: Repo[]) => reposD.setDraft(JSON.stringify(r));

  const fields = [nameD, colorD, stateD, descD, reposD];
  const submitting = useSubmitting(dk);

  const finish = () => {
    dropDrafts(dk);
    onDone();
  };

  // Same rules as the topic editor (lib/form.ts): changed fields only, compared
  // with what will be stored; one submission at a time; close only when the
  // request succeeded and nothing was typed meanwhile.
  const save = async () => {
    const built = projectPatch({
      name: { edited: nameD.edited(), stored: nameD.stored() },
      color: { edited: colorD.edited(), stored: colorD.stored() },
      state: { edited: stateD.edited(), stored: stateD.stored() },
      description: { edited: descD.edited(), stored: descD.stored() },
      repos: { edited: reposD.edited(), stored: reposD.stored() },
    });
    if (submitting.isBusy()) return;
    if ("error" in built) {
      notify(built.error);
      return;
    }
    const { patch } = built;
    const action = submitAction(submitting.isBusy(), patch);
    if (action === "ignore") return;
    if (action === "close") return finish();
    if (!submitting.begin()) return;
    try {
      const call = once(() => run((a) => a.updateProject(project.slug, patch)));
      const acked = [
        patch.name !== undefined && nameD.save(patch.name, call, (p) => p.name),
        patch.color !== undefined && colorD.save(patch.color ?? "", call, (p) => p.color ?? ""),
        patch.state !== undefined && stateD.save(patch.state, call, (p) => p.state),
        patch.description !== undefined && descD.save(patch.description, call, (p) => p.description),
        patch.repos !== undefined && reposD.save(JSON.stringify(patch.repos), call, (p) => JSON.stringify(p.repos)),
      ];
      const ok = (await call()) !== undefined;
      await Promise.all(acked); // the fields' own results are applied
      if (closeAfterSave(ok, fields.map((f) => f.state()))) finish();
    } finally {
      submitting.end();
    }
  };
  const cancel = () => !submitting.isBusy() && finish();

  return (
    <form
      className="editor project-editor"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => e.key === "Escape" && cancel()}
    >
      <label className="d-label" htmlFor="pe-name">
        Nazwa
      </label>
      <input
        id="pe-name"
        className="field"
        value={name}
        autoFocus
        onChange={(e) => nameD.setDraft(e.target.value)}
        onFocus={nameD.onFocus}
        onBlur={nameD.onBlur}
      />
      <DraftHint draft={nameD} what="Nazwa" />

      <span className="d-label">Kolor</span>
      <div className="swatches" role="radiogroup" aria-label="Kolor projektu">
        {PROJECT_COLORS.map((c) => (
          <button
            key={c}
            type="button"
            role="radio"
            aria-checked={color.toLowerCase() === c}
            aria-label={c}
            className="swatch"
            style={{ background: c }}
            onClick={() => colorD.setDraft(c)}
          />
        ))}
        <button
          type="button"
          role="radio"
          aria-checked={!color}
          aria-label="Bez koloru"
          title="Bez koloru"
          className="swatch none"
          onClick={() => colorD.setDraft("")}
        />
        <input
          className="field field-hex"
          aria-label="Własny kolor (#rrggbb)"
          placeholder="#rrggbb"
          value={color}
          onChange={(e) => colorD.setDraft(e.target.value.trim())}
          onFocus={colorD.onFocus}
          onBlur={colorD.onBlur}
        />
      </div>
      <DraftHint draft={colorD} what="Kolor" />

      <span className="d-label">Stan</span>
      <Segmented<ProjectState>
        label="Stan projektu"
        value={state}
        onChange={(s) => stateD.setDraft(s)}
        options={PROJECT_STATES.map((s) => ({ value: s, label: PROJECT_STATE_LABEL[s] }))}
      />
      <DraftHint draft={stateD} what="Stan" />

      <label className="d-label" htmlFor="pe-desc">
        Opis
      </label>
      <textarea
        id="pe-desc"
        className="field"
        rows={3}
        value={description}
        placeholder="Po co jest ten projekt?"
        onChange={(e) => descD.setDraft(e.target.value)}
        onFocus={descD.onFocus}
        onBlur={descD.onBlur}
      />
      <DraftHint draft={descD} what="Opis" />

      <span className="d-label">Repozytoria</span>
      <div className="repo-list" onFocus={reposD.onFocus} onBlur={reposD.onBlur}>
        {repos.map((r, i) => (
          <div className="repo-row" key={i}>
            <input
              className="field"
              placeholder="nazwa"
              aria-label="Nazwa repozytorium"
              value={r.name}
              onChange={(e) => setRepos(repos.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)))}
            />
            <input
              className="field"
              placeholder="/ścieżka/do/repo"
              aria-label="Ścieżka repozytorium"
              value={r.path}
              onChange={(e) => setRepos(repos.map((x, j) => (j === i ? { ...x, path: e.target.value } : x)))}
            />
            <IconButton label="Usuń repozytorium" onClick={() => setRepos(repos.filter((_, j) => j !== i))}>
              <X size={13} />
            </IconButton>
          </div>
        ))}
        <Button variant="quiet" className="add-repo" onClick={() => setRepos([...repos, { name: "", path: "" }])}>
          + Dodaj repozytorium
        </Button>
      </div>
      <DraftHint draft={reposD} what="Repozytoria" />

      <SaveActions busy={submitting.busy} onCancel={cancel} />
    </form>
  );
}

export function ProjectHeader({ project }: { project: Project }) {
  const { run } = useVault();
  const { reveal } = useActions();
  const [editing, setEditing] = useState(false);

  if (editing) {
    return (
      <header className="view-head">
        <h1 className="view-title">
          <ProjectDot color={project.color} size={10} />
          {project.name}
        </h1>
        <ProjectEditor project={project} onDone={() => setEditing(false)} />
      </header>
    );
  }

  return (
    <header className="view-head">
      <h1 className="view-title">
        <ProjectDot color={project.color} size={10} />
        <span className="title-text">{project.name}</span>
        {project.state !== "active" && <span className={`state-tag state-${project.state}`}>{PROJECT_STATE_LABEL[project.state]}</span>}
      </h1>
      {project.description && <p className="view-desc">{project.description}</p>}
      {project.repos.length > 0 && (
        <ul className="repos" aria-label="Repozytoria">
          {project.repos.map((r) => (
            <li key={r.name} title={r.path}>
              <FolderGit2 size={12} />
              {r.name}
            </li>
          ))}
        </ul>
      )}
      <div className="head-actions">
        <Button variant="quiet" onClick={() => setEditing(true)}>
          Edytuj projekt
        </Button>
        <Button variant="quiet" onClick={() => void reveal(project.path)}>
          <FileText size={13} /> Pokaż plik
        </Button>
        {project.state !== "archived" ? (
          <ConfirmButton
            question="Przenieść projekt do archiwum?"
            confirmLabel="Archiwizuj"
            onConfirm={() => void run((a) => a.archiveProject(project.slug))}
          >
            Archiwizuj
          </ConfirmButton>
        ) : (
          <Button variant="quiet" onClick={() => void run((a) => a.updateProject(project.slug, { state: "active" }))}>
            Przywróć z archiwum
          </Button>
        )}
      </div>
    </header>
  );
}
