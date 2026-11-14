// „Akcje": prepared agent recipes, run in the background (progress and answer
// stream into a panel) or in Terminal.app. Run state lives in store/runs.ts.

import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { api, mockQuery, type ProviderId, type ProviderInfo, type Recipe, type RecipeParam, type RunMode } from "../../api";
import { Button } from "../../components/ui";
import {
  MODE_LABEL,
  STATUS_LABEL,
  buildRequest,
  formatRunTime,
  initialValues,
  pickProvider,
  validateParams,
  type ParamValues,
} from "../../lib/actions";
import { describeError } from "../../lib/errors";
import { projectProblem } from "../../lib/updates";
import { partitionProjects } from "../../lib/group";
import { readPref, writePref } from "../../store/ui";
import { runStore, useRuns } from "../../store/runs";
import { useVault, useView } from "../../store/vault";
import { RECIPE_PREF, takePreset } from "./preset";
import { PROVIDER_NAME, RunPanel } from "./RunPanel";

const providerPref = (recipe: string) => `morwisko.akcje.provider.${recipe}`;

// Survive navigating away and back (module scope, like store/ui.ts).
const formMemory = new Map<string, ParamValues>();
let catalog: { recipes: Recipe[]; providers: ProviderInfo[] } | null = null;

function AutoTextarea({ value, onChange, placeholder, id, invalid }: { value: string; onChange: (v: string) => void; placeholder: string | null; id: string; invalid: boolean }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight + 2, 280)}px`;
  }, [value]);
  return (
    <textarea
      ref={ref}
      id={id}
      className="field act-textarea"
      rows={3}
      value={value}
      placeholder={placeholder ?? undefined}
      aria-invalid={invalid || undefined}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

function ParamField({
  param,
  value,
  onChange,
  projects,
  error,
}: {
  param: RecipeParam;
  value: string;
  onChange: (v: string) => void;
  projects: { slug: string; name: string; paused: boolean }[];
  error: string | undefined;
}) {
  const id = `act-p-${param.key}`;
  return (
    <div className="act-field">
      <label className="act-label" htmlFor={id}>
        {param.label}
        {!param.required && <span className="act-opt"> · opcjonalnie</span>}
      </label>
      {param.kind === "project" && (
        <select id={id} className="field select act-select" value={value} aria-invalid={!!error || undefined} onChange={(e) => onChange(e.target.value)}>
          <option value="">{param.required ? "Wybierz projekt…" : "Wszystkie aktywne projekty"}</option>
          {projects.map((p) => (
            <option key={p.slug} value={p.slug}>
              {p.name}
              {p.paused ? " (wstrzymany)" : ""}
            </option>
          ))}
        </select>
      )}
      {param.kind === "period" && (
        <div className="segmented" role="radiogroup" aria-label={param.label} id={id}>
          {[
            ["dzis", "Dziś"],
            ["tydzien", "Tydzień"],
          ].map(([v, label]) => (
            <button key={v} type="button" role="radio" aria-checked={value === v} className={value === v ? "on" : undefined} onClick={() => onChange(v!)}>
              {label}
            </button>
          ))}
        </div>
      )}
      {param.kind === "text" && <AutoTextarea id={id} value={value} onChange={onChange} placeholder={param.placeholder} invalid={!!error} />}
      {error && (
        <p className="act-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function ProviderSwitch({ providers, value, onChange }: { providers: ProviderInfo[]; value: ProviderId | null; onChange: (p: ProviderId) => void }) {
  const broken = providers.filter((p) => !p.available);
  return (
    <div className="act-field">
      <span className="act-label" id="act-provider-label">
        Subskrypcja
      </span>
      <div className="segmented" role="radiogroup" aria-labelledby="act-provider-label">
        {providers.map((p) => (
          <button
            key={p.id}
            type="button"
            role="radio"
            aria-checked={value === p.id}
            disabled={!p.available}
            title={p.available ? (p.version ? `${p.name} ${p.version}` : p.name) : (p.problem ?? "Niedostępny")}
            className={value === p.id ? "on" : undefined}
            onClick={() => onChange(p.id)}
          >
            {p.name}
          </button>
        ))}
      </div>
      {broken.map((p) => (
        <p key={p.id} className="act-hint">
          {p.name} niedostępny: {p.problem ?? "nie znaleziono polecenia."}
        </p>
      ))}
    </div>
  );
}

function RecipeForm({ recipe, providers, runningBackground }: { recipe: Recipe; providers: ProviderInfo[]; runningBackground: boolean }) {
  const view = useView();
  const { notify } = useVault();
  const projects = partitionProjects(view.snapshot.projects)
    .visible.filter((p) => p.state !== "archived")
    .map((p) => ({ slug: p.slug, name: p.name, paused: p.state === "paused", repos: p.repos }));

  const [values, setValues] = useState<ParamValues>(() => {
    const preset = takePreset(recipe.id);
    const base = formMemory.get(recipe.id) ?? initialValues(recipe);
    if (!preset) return base;
    const next = { ...base, ...preset };
    formMemory.set(recipe.id, next);
    return next;
  });
  const [provider, setProvider] = useState<ProviderId | null>(() => pickProvider(providers, readPref(providerPref(recipe.id))));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState<RunMode | null>(null);

  // A provider that became (un)available meanwhile.
  useEffect(() => {
    setProvider((cur) => (cur && providers.find((p) => p.id === cur)?.available ? cur : pickProvider(providers, readPref(providerPref(recipe.id)))));
  }, [providers, recipe.id]);

  const set = (key: string, v: string) => {
    setValues((cur) => {
      const next = { ...cur, [key]: v };
      formMemory.set(recipe.id, next);
      return next;
    });
    setErrors((e) => {
      if (!(key in e)) return e;
      const { [key]: _gone, ...rest } = e;
      void _gone;
      return rest;
    });
  };

  async function start(mode: RunMode) {
    setFailure(null);
    if (!provider) return;
    const errs = validateParams(recipe, values, projects.map((p) => p.slug));
    const projectParam = recipe.params.find((p) => p.kind === "project");
    if (projectParam && !errs[projectParam.key]) {
      const problem = projectProblem(recipe, projects.find((p) => p.slug === values[projectParam.key]));
      if (problem) errs[projectParam.key] = problem;
    }
    setErrors(errs);
    if (Object.keys(errs).length) return;
    setBusy(mode);
    try {
      await runStore.startRun(buildRequest(recipe, provider, mode, values));
    } catch (e) {
      const msg = describeError(e);
      setFailure(msg);
      notify(msg);
    } finally {
      setBusy(null);
    }
  }

  const hasBackground = recipe.modes.includes("background");
  const hasTerminal = recipe.modes.includes("terminal");
  const noProvider = provider === null;
  const backgroundBlocked = runningBackground ? "Inna akcja w tle jest w toku — poczekaj na jej koniec albo ją przerwij." : null;

  return (
    <section className="act-form" aria-label={recipe.title}>
      <h2 className="act-form-title">{recipe.title}</h2>
      <p className="act-form-desc">{recipe.description}</p>

      {recipe.params.map((p) => (
        <ParamField key={p.key} param={p} value={values[p.key] ?? ""} onChange={(v) => set(p.key, v)} projects={projects} error={errors[p.key]} />
      ))}

      <ProviderSwitch
        providers={providers}
        value={provider}
        onChange={(id) => {
          setProvider(id);
          writePref(providerPref(recipe.id), id);
        }}
      />

      <div className="act-buttons">
        {hasBackground && (
          <Button variant="primary" disabled={noProvider || runningBackground || busy !== null} onClick={() => void start("background")}>
            {busy === "background" ? "Uruchamiam…" : "Uruchom w tle"}
          </Button>
        )}
        {hasTerminal && (
          <Button variant="ghost" disabled={noProvider || busy !== null} onClick={() => void start("terminal")}>
            {busy === "terminal" ? "Otwieram…" : "Otwórz w terminalu"}
          </Button>
        )}
      </div>
      <div className="act-notes">
        {noProvider && <p className="act-hint">Żaden agent nie jest dostępny — zainstaluj Claude Code albo Codex i zaloguj się.</p>}
        {!hasBackground && <p className="act-hint">Ta akcja działa tylko w terminalu: tam możesz odpowiadać agentowi na bieżąco.</p>}
        {hasBackground && backgroundBlocked && <p className="act-hint">{backgroundBlocked}</p>}
        {recipe.proposes && <p className="act-hint">Propozycje pojawią się w widoku projektu, w pasku Aktualizacja.</p>}
        {recipe.background_writes && (
          <p className="act-hint act-warn">
            Uruchomiona w tle, ta akcja dodaje zadania w Morwisku bez pytania. W terminalu agent najpierw je zaproponuje.
          </p>
        )}
        {failure && (
          <p className="act-error" role="alert">
            {failure}
          </p>
        )}
      </div>
    </section>
  );
}

export function ActionsView() {
  const runs = useRuns();
  const [cat, setCat] = useState(catalog);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [recipeId, setRecipeId] = useState<string | null>(() => mockQuery("recipe") ?? readPref(RECIPE_PREF));

  useEffect(() => {
    let live = true;
    Promise.all([api.listRecipes(), api.listProviders()]).then(
      ([recipes, providers]) => {
        catalog = { recipes, providers };
        if (live) {
          setCat(catalog);
          setLoadError(null);
        }
      },
      (e) => live && setLoadError(describeError(e)),
    );
    // The store loads the list too; this repairs a failed first attempt.
    void runStore.start().catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const recipe = cat?.recipes.find((r) => r.id === recipeId) ?? cat?.recipes[0] ?? null;
  const shown = runs.data.runs[runs.selectedId ?? ""] ?? runs.active ?? null;

  return (
    <div className="view actions">
      <header className="view-head">
        <h1 className="view-title">Akcje</h1>
        <p className="view-sub">Gotowe przepisy dla agenta. Uruchom w tle albo w terminalu, na wybranej subskrypcji.</p>
      </header>

      {loadError && !cat && (
        <div className="empty compact">
          <p className="empty-title">Nie udało się wczytać akcji.</p>
          <p className="empty-text">{loadError}</p>
        </div>
      )}

      {cat && (
        <div className="act-layout">
          <aside className="act-side">
            <div className="group-head">Przepisy</div>
            <div className="act-recipes" role="group" aria-label="Przepisy">
              {cat.recipes.map((r) => (
                <button
                  key={r.id}
                  type="button"
                  aria-pressed={r.id === recipe?.id}
                  aria-label={r.title}
                  aria-describedby={`act-recipe-desc-${r.id}`}
                  className={`act-recipe${r.id === recipe?.id ? " on" : ""}`}
                  onClick={() => {
                    setRecipeId(r.id);
                    writePref(RECIPE_PREF, r.id);
                  }}
                >
                  <span className="act-recipe-title">{r.title}</span>
                  <span className="act-recipe-desc" id={`act-recipe-desc-${r.id}`}>
                    {r.description}
                  </span>
                  <span className="act-badges">
                    {r.modes.map((m) => (
                      <span key={m} className="act-badge">
                        {MODE_LABEL[m]}
                      </span>
                    ))}
                    {r.background_writes && <span className="act-badge warn">zapisuje w Morwisku</span>}
                    {r.proposes && <span className="act-badge">zapis po akceptacji</span>}
                  </span>
                </button>
              ))}
            </div>

            <div className="group-head act-recent-head">Ostatnie uruchomienia</div>
            {runs.list.length === 0 ? (
              <p className="act-hint">Jeszcze nic nie uruchomiono.</p>
            ) : (
              <ul className="act-runs">
                {runs.list.map((r) => (
                  <li key={r.id}>
                    <button type="button" className={`act-run${r.id === shown?.id ? " on" : ""}`} onClick={() => runStore.select(r.id)}>
                      <span className={`act-dot st-${r.status}`} aria-hidden />
                      <span className="act-run-main">
                        <span className="act-run-title">{r.recipe_title}</span>
                        <span className="act-run-sub">
                          {PROVIDER_NAME[r.provider]} · {MODE_LABEL[r.mode]} · {r.mode === "terminal" ? "otwarto" : STATUS_LABEL[r.status].toLowerCase()}
                        </span>
                      </span>
                      <time className="act-run-time">{formatRunTime(r.started_at)}</time>
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <p className="act-hint act-tiny">Lista znika po zamknięciu aplikacji.</p>
          </aside>

          <div className="act-main">
            {recipe && <RecipeForm key={recipe.id} recipe={recipe} providers={cat.providers} runningBackground={runs.active !== null} />}
            {shown && <RunPanel key={shown.id} run={shown} recipe={cat.recipes.find((r) => r.id === shown.recipe) ?? null} />}
          </div>
        </div>
      )}
    </div>
  );
}
