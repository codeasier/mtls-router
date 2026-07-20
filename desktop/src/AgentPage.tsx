import {
  useEffect,
  useEffectEvent,
  useRef,
  useState,
  type ComponentPropsWithoutRef,
} from "react";

import { useI18n } from "./i18n";
import {
  sanitizeSensitiveText,
  initializeAgentConfig,
  type AgentDetection,
  type AgentFileEffect,
  type AgentId,
  type AgentModelsResult,
  type AgentPreview,
  type AgentState,
  type AgentWriteResult,
  type DesktopApi,
  type JsonObject,
  type ModelConfig,
  type ModelSelection,
  type InitializationSource,
  type OpenCodeModelConfig,
} from "./ipc";

type Stage =
  | "select"
  | "credential"
  | "discover"
  | "configure"
  | "preview"
  | "write"
  | "result";
const agentOrder: AgentId[] = ["claude", "opencode", "codex"];
const agentNames: Record<AgentId, string> = {
  claude: "Claude Code",
  opencode: "opencode",
  codex: "Codex",
};
const roleNames = ["haiku", "sonnet", "opus"] as const;

function safe(value: string | undefined) {
  return sanitizeSensitiveText(value ?? "");
}
function errorCode(error: unknown) {
  return typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof (error as { code?: unknown }).code === "string"
    ? (error as { code: string }).code
    : "";
}
function selectable(agent: AgentState) {
  return agent.detected && agent.writable && !agent.invalid;
}
function initialSelection(detection: AgentDetection) {
  return detection.agents.filter(selectable).map((agent) => agent.agent);
}
function ClaudeSelectionFields({
  id,
  selection,
  models,
  modelLabel,
  onChange,
}: {
  id: string;
  selection: ModelSelection;
  models: string[];
  modelLabel: string;
  onChange: (selection: ModelSelection) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="claude-selection-fields">
      <label className="option-field">
        <span>{modelLabel}</span>
        <AgentSelect
          value={selection.model}
          onChange={(event) => onChange({ model: event.target.value })}
        >
          <option value="">{t("agents.chooseModel")}</option>
          {models.map((model) => (
            <option key={model}>{model}</option>
          ))}
        </AgentSelect>
      </label>
      <label className="option-field">
        <span>{t("agents.displayName")}</span>
        <input
          aria-label={`${id} ${t("agents.displayName")}`}
          value={selection.name ?? ""}
          placeholder={t("agents.unset")}
          onChange={(event) =>
            onChange({ ...selection, name: event.target.value || undefined })
          }
        />
      </label>
      <fieldset className="context-selector">
        <legend>{t("agents.contextMode")}</legend>
        <label>
          <input
            type="radio"
            name={`${id}-context`}
            checked={!selection.context}
            onChange={() => onChange({ ...selection, context: undefined })}
          />
          {t("agents.contextStandard")}
        </label>
        <label>
          <input
            type="radio"
            name={`${id}-context`}
            checked={selection.context === "1m"}
            onChange={() => onChange({ ...selection, context: "1m" })}
          />
          {t("agents.context1m")}
        </label>
      </fieldset>
    </div>
  );
}

function AgentSelect({
  children,
  ...props
}: ComponentPropsWithoutRef<"select">) {
  return (
    <span className="agent-select-control">
      <select {...props}>{children}</select>
      <span className="agent-select-control__chevron" aria-hidden="true" />
    </span>
  );
}

function parseExtra(text: string): { value?: JsonObject; error?: string } {
  if (!text.trim()) return {};
  try {
    const value: unknown = JSON.parse(text);
    if (!value || Array.isArray(value) || typeof value !== "object")
      return { error: "/: object_required" };
    const protectedKey =
      /(credential|apikey|auth|token|secret|password|bearer|header|url|endpoint|provider|connection|transport|proxy|fetch)/i;
    const visit = (
      current: unknown,
      path: string,
      depth: number,
    ): string | undefined => {
      if (depth > 16) return `${path}: max_depth`;
      if (!current || Array.isArray(current) || typeof current !== "object")
        return undefined;
      for (const [key, child] of Object.entries(current)) {
        if (protectedKey.test(key.replace(/[_.-]/g, "")))
          return `${path}/${key}: protected_path`;
        if (child === null) return `${path}/${key}: non_null`;
        const nested = visit(child, `${path}/${key}`, depth + 1);
        if (nested) return nested;
      }
    };
    const error = visit(value, "", 0);
    return error ? { error } : { value: value as JsonObject };
  } catch {
    return { error: "/: valid_json" };
  }
}

function Catalog({
  models,
  search,
  setSearch,
  selected,
  onSelect,
  multiple = false,
}: {
  models: string[];
  search: string;
  setSearch: (value: string) => void;
  selected: string[];
  onSelect: (model: string) => void;
  multiple?: boolean;
}) {
  const { t } = useI18n();
  const visible = models.filter((model) =>
    model.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
  );
  return (
    <div className="model-catalog">
      <label className="catalog-search">
        <span>{t("agents.catalogSearch")}</span>
        <input
          type="search"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
      </label>
      <div
        className="catalog-list"
        role={multiple ? "group" : "radiogroup"}
        aria-label={t("agents.catalogLabel")}
      >
        {visible.map((model) => (
          <label
            key={model}
            className={
              selected.includes(model)
                ? "catalog-model is-selected"
                : "catalog-model"
            }
          >
            <input
              type={multiple ? "checkbox" : "radio"}
              checked={selected.includes(model)}
              onChange={() => onSelect(model)}
            />
            <code>{safe(model)}</code>
          </label>
        ))}
        {!visible.length && <p>{t("agents.catalogEmptySearch")}</p>}
      </div>
    </div>
  );
}

function ExtraEditor({
  agent,
  value,
  onChange,
}: {
  agent: AgentId;
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useI18n();
  const parsed = parseExtra(value);
  return (
    <details className="advanced-editor">
      <summary>{t("agents.advancedExtra")}</summary>
      <label>
        <span>{t("agents.extraJson", { agent: agentNames[agent] })}</span>
        <textarea
          aria-invalid={Boolean(parsed.error)}
          aria-describedby={`${agent}-extra-error`}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          spellCheck={false}
        />
      </label>
      <div className="editor-actions">
        <button
          type="button"
          className="text-button"
          disabled={!parsed.value}
          onClick={() => onChange(JSON.stringify(parsed.value, null, 2))}
        >
          {t("agents.formatJson")}
        </button>
        <small
          id={`${agent}-extra-error`}
          role={parsed.error ? "alert" : undefined}
        >
          {parsed.error ?? t("agents.extraValid")}
        </small>
      </div>
    </details>
  );
}

function OptionalSelect({
  label,
  value,
  values,
  onChange,
}: {
  label: string;
  value?: string;
  values: string[];
  onChange: (value?: string) => void;
}) {
  const { t } = useI18n();
  return (
    <label className="option-field">
      <span>{label}</span>
      <AgentSelect
        value={value ?? ""}
        onChange={(event) => onChange(event.target.value || undefined)}
      >
        <option value="">{t("agents.unset")}</option>
        {values.map((item) => (
          <option key={item} value={item}>
            {item}
          </option>
        ))}
      </AgentSelect>
    </label>
  );
}

function OptionalNumber({
  label,
  value,
  onChange,
}: {
  label: string;
  value?: number;
  onChange: (value?: number) => void;
}) {
  return (
    <label className="option-field">
      <span>{label}</span>
      <input
        type="number"
        min="1"
        max="9007199254740991"
        value={value ?? ""}
        onChange={(event) =>
          onChange(event.target.value ? Number(event.target.value) : undefined)
        }
      />
    </label>
  );
}

function ObjectField({
  label,
  value,
  onChange,
  onErrorChange,
}: {
  label: string;
  value?: JsonObject;
  onChange: (value?: JsonObject) => void;
  onErrorChange: (error?: string) => void;
}) {
  const { t } = useI18n();
  const serialized = value ? JSON.stringify(value, null, 2) : "";
  const [text, setText] = useState(serialized);
  const locallyEmitted = useRef<JsonObject | undefined>(undefined);
  const reportError = useEffectEvent(onErrorChange);
  useEffect(() => {
    if (value !== locallyEmitted.current) {
      setText(serialized);
      reportError(undefined);
    }
    locallyEmitted.current = undefined;
  }, [serialized, value]);
  useEffect(() => () => reportError(undefined), []);
  const parsed = parseExtra(text);
  return (
    <div className="object-field">
      <label>
        <span>{label}</span>
        <textarea
          aria-invalid={Boolean(parsed.error)}
          value={text}
          placeholder={t("agents.unset")}
          onChange={(event) => {
            const next = event.target.value;
            setText(next);
            const result = parseExtra(next);
            onErrorChange(result.error);
            if (!next.trim()) {
              locallyEmitted.current = undefined;
              onChange(undefined);
            } else if (result.value) {
              locallyEmitted.current = result.value;
              onChange(result.value);
            }
          }}
          spellCheck={false}
        />
      </label>
      <button
        type="button"
        className="text-button"
        disabled={!parsed.value}
        onClick={() => setText(JSON.stringify(parsed.value, null, 2))}
      >
        {t("agents.formatJson")}
      </button>
      <small role={parsed.error ? "alert" : undefined}>
        {parsed.error ?? t("agents.extraValid")}
      </small>
    </div>
  );
}

function FileEffect({ effect }: { effect: AgentFileEffect }) {
  return (
    <article className="effect-card">
      <span>{safe(effect.operation).toUpperCase()}</span>
      <strong>{safe(effect.role)}</strong>
      <code>{safe(effect.path)}</code>
      {effect.backup_path && (
        <code className="backup-path">{safe(effect.backup_path)}</code>
      )}
    </article>
  );
}

export function AgentPage({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [detection, setDetection] = useState<AgentDetection | null>(null);
  const [selected, setSelected] = useState<AgentId[]>([]);
  const [stage, setStage] = useState<Stage>("select");
  const [key, setKey] = useState("");
  const [discovery, setDiscovery] = useState<AgentModelsResult | null>(null);
  const [config, setConfig] = useState<ModelConfig>({ version: 1 });
  const [sources, setSources] = useState<Record<AgentId, InitializationSource>>(
    { claude: "empty", opencode: "empty", codex: "empty" },
  );
  const [preview, setPreview] = useState<AgentPreview | null>(null);
  const [result, setResult] = useState<AgentWriteResult | null>(null);
  const [search, setSearch] = useState("");
  const [extras, setExtras] = useState<Record<AgentId, string>>({
    claude: "",
    opencode: "",
    codex: "",
  });
  const [objectFieldErrors, setObjectFieldErrors] = useState<
    Record<string, string>
  >({});
  const [approveDrift, setApproveDrift] = useState(false);
  const [approveAuth, setApproveAuth] = useState(false);
  const [busy, setBusy] = useState(true);
  const [message, setMessage] = useState("");
  const flowRef = useRef("");

  async function destroyFlow() {
    const flow = flowRef.current;
    flowRef.current = "";
    if (flow) await api.destroyAgentModelFlow(flow).catch(() => undefined);
  }
  useEffect(() => {
    let active = true;
    void api
      .detectAgents()
      .then((value) => {
        if (active) {
          setDetection(value);
          setSelected(initialSelection(value));
        }
      })
      .catch(() => active && setMessage(t("agents.error.detect")))
      .finally(() => active && setBusy(false));
    return () => {
      active = false;
      const flow = flowRef.current;
      flowRef.current = "";
      if (flow) void api.destroyAgentModelFlow(flow);
    };
  }, [api, t]);

  function toggleAgent(agent: AgentId) {
    setSelected((current) =>
      current.includes(agent)
        ? current.filter((item) => item !== agent)
        : agentOrder.filter((item) => item === agent || current.includes(item)),
    );
  }
  async function startOver(target: Stage = "select") {
    await destroyFlow();
    setKey("");
    setDiscovery(null);
    setPreview(null);
    setResult(null);
    setApproveDrift(false);
    setApproveAuth(false);
    setMessage("");
    setStage(target);
  }
  async function discover(event: React.FormEvent) {
    event.preventDefault();
    if (!key || !selected.length) return;
    const transient = key;
    setKey("");
    setBusy(true);
    setMessage("");
    setStage("discover");
    try {
      const value = await api.discoverModels(selected, transient);
      flowRef.current = value.flow_id;
      setDiscovery(value);
      const initialized = initializeAgentConfig(
        selected,
        value.existing.model_config,
        value.preset.model_config,
      );
      setConfig(initialized.config);
      setSources(initialized.sources);
      setStage("configure");
    } catch (error) {
      setMessage(
        t(
          errorCode(error) === "MODEL_AUTH_FAILED"
            ? "agents.error.auth"
            : "agents.error.discovery",
        ),
      );
      setStage("credential");
    } finally {
      setBusy(false);
    }
  }
  function configError(): string {
    if (!discovery) return "/: catalog_required";
    if (config.claude && !config.claude.primary.model)
      return "/claude/primary/model: required";
    if (config.claude) {
      for (const role of [
        ...roleNames,
        ...(config.claude.fable ? ["fable" as const] : []),
      ]) {
        const selection = config.claude[role];
        if (selection && !("inherit_primary" in selection) && !selection.model)
          return `/claude/${role}/model: required`;
      }
    }
    if (
      config.opencode &&
      (!Object.keys(config.opencode.models).length ||
        !config.opencode.default_model)
    )
      return "/opencode/default_model: required";
    if (config.codex && !config.codex.model) return "/codex/model: required";
    for (const agent of selected) {
      const parsed = parseExtra(extras[agent]);
      if (parsed.error) return `/${agent}/extra${parsed.error}`;
    }
    const objectError = Object.entries(objectFieldErrors)[0];
    if (objectError)
      return `/opencode/models/${objectError[0]}: ${objectError[1]}`;
    return "";
  }
  function setObjectFieldError(path: string, error?: string) {
    setObjectFieldErrors((current) => {
      if (error) return { ...current, [path]: error };
      if (!(path in current)) return current;
      const next = { ...current };
      delete next[path];
      return next;
    });
  }
  function finalConfig(): ModelConfig {
    const value = structuredClone(config);
    for (const agent of selected) {
      const parsed = parseExtra(extras[agent]).value;
      if (!parsed || !Object.keys(parsed).length) continue;
      if (agent === "claude" && value.claude)
        value.claude.extra = Object.fromEntries(
          Object.entries(parsed).map(([key, child]) => [key, String(child)]),
        );
      if (agent === "codex" && value.codex)
        value.codex.extra = parsed as NonNullable<typeof value.codex.extra>;
    }
    return value;
  }
  async function loadPreview() {
    if (!discovery) return;
    const invalid = configError();
    if (invalid) {
      setMessage(invalid);
      return;
    }
    setBusy(true);
    setMessage("");
    try {
      const value = await api.previewAgents(
        selected,
        discovery.flow_id,
        discovery.catalog_token,
        finalConfig(),
      );
      setPreview(value);
      setConfig(value.model_config);
      setStage("preview");
    } catch (error) {
      const code = errorCode(error);
      if (code === "MODEL_CATALOG_STALE") {
        await startOver("credential");
        setMessage(t("agents.error.catalogStale"));
      } else
        setMessage(
          t("agents.error.config", {
            detail: safe(
              (error as { details?: { path?: string; rule?: string } })?.details
                ?.path,
            ),
          }),
        );
    } finally {
      setBusy(false);
    }
  }
  async function write() {
    if (!discovery || !preview) return;
    setBusy(true);
    setStage("write");
    setMessage("");
    try {
      const value = await api.writeAgents(
        selected,
        discovery.flow_id,
        discovery.catalog_token,
        config,
        preview.revision_token,
        approveDrift,
        approveAuth,
      );
      flowRef.current = "";
      setResult(value);
      setStage("result");
    } catch (error) {
      const code = errorCode(error);
      if (code === "PREVIEW_STALE") {
        setStage("configure");
        setMessage(t("agents.previewRefreshed"));
      } else if (
        code === "MODEL_NOT_AVAILABLE" ||
        code === "MODEL_CATALOG_STALE"
      ) {
        await startOver("credential");
        setMessage(t("agents.error.catalogStale"));
      } else {
        await destroyFlow();
        setStage("credential");
        setMessage(t("agents.error.write"));
      }
    } finally {
      setBusy(false);
    }
  }
  async function importConfig(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file || !discovery) return;
    if (
      file.size > 2 * 1024 * 1024 ||
      !file.name.toLowerCase().endsWith(".json")
    ) {
      setMessage(t("agents.error.import"));
      return;
    }
    try {
      setConfig(
        await api.importAgentModelConfig(
          await file.text(),
          selected,
          discovery.flow_id,
        ),
      );
      setExtras({ claude: "", opencode: "", codex: "" });
      setObjectFieldErrors({});
      setSources({ claude: "empty", opencode: "empty", codex: "empty" });
      setMessage(t("agents.imported"));
    } catch {
      setMessage(t("agents.error.import"));
    }
  }
  async function exportConfig() {
    if (!discovery || configError()) return;
    try {
      const content = await api.exportAgentModelConfig(
        finalConfig(),
        selected,
        discovery.flow_id,
      );
      const url = URL.createObjectURL(
        new Blob([content], { type: "application/json" }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = "mtls-router-model-config.json";
      link.click();
      URL.revokeObjectURL(url);
    } catch {
      setMessage(t("agents.error.export"));
    }
  }

  const byAgent = new Map(
    detection?.agents.map((agent) => [agent.agent, agent]) ?? [],
  );
  const invalid = configError();
  return (
    <section className="agents-workbench" aria-labelledby="agents-heading">
      <header className="agents-workbench__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 id="agents-heading">{t("agents.heading")}</h2>
        </div>
        <div
          className="stage-meter"
          aria-label={t("agents.currentStage", {
            stage: t(`agents.stage.${stage}`),
          })}
        >
          {(
            [
              "select",
              "credential",
              "discover",
              "configure",
              "preview",
              "write",
              "result",
            ] as Stage[]
          ).map((item, index) => (
            <span key={item} className={item === stage ? "is-current" : ""}>
              {String(index + 1).padStart(2, "0")}
            </span>
          ))}
        </div>
      </header>
      {message && (
        <p className="agent-alert" role="alert">
          {message}
        </p>
      )}
      {stage === "select" && (
        <>
          <div className="agent-toolbar">
            <p>{t("agents.selectionNote")}</p>
          </div>
          <div className="agent-card-grid">
            {agentOrder.map((id) => {
              const agent = byAgent.get(id);
              return (
                <article
                  className={`agent-card${selected.includes(id) ? " is-selected" : ""}`}
                  key={id}
                >
                  <h3>{agentNames[id]}</h3>
                  <p
                    className="agent-card__config-path"
                    title={agent ? safe(agent.path) : undefined}
                  >
                    {agent ? safe(agent.path) : t("agents.noResult")}
                  </p>
                  <span className="agent-state">
                    {agent?.configured
                      ? t("agents.detection.configured")
                      : t("agents.detection.ready")}
                  </span>
                  <label className="agent-select">
                    <input
                      type="checkbox"
                      checked={selected.includes(id)}
                      disabled={!agent || !selectable(agent)}
                      onChange={() => toggleAgent(id)}
                    />
                    <span>{t("agents.select")}</span>
                  </label>
                </article>
              );
            })}
          </div>
          <div className="agent-footer-action">
            <span>{t("agents.selectedCount", { count: selected.length })}</span>
            <button
              className="control-button"
              disabled={!selected.length}
              onClick={() => setStage("credential")}
            >
              {t("agents.continue")}
            </button>
          </div>
        </>
      )}
      {stage === "credential" && (
        <form className="credential-stage" onSubmit={discover}>
          <p className="overline">{t("agents.stage.credential")}</p>
          <h3>{t("agents.credentialHeading")}</h3>
          <p>{t("agents.credentialNote")}</p>
          <label className="key-field">
            <span>{t("agents.apiKey")}</span>
            <input
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
          </label>
          <div className="action-row">
            <button className="control-button" disabled={!key || busy}>
              {t("agents.discover")}
            </button>
            <button
              type="button"
              className="text-button"
              onClick={() => void startOver()}
            >
              {t("agents.cancelDetection")}
            </button>
          </div>
        </form>
      )}
      {(stage === "discover" || stage === "write") && (
        <div className="processing-stage" role="status">
          <span className="instrument__dial">
            {stage === "discover" ? "GET" : "TX"}
          </span>
          <h3>
            {t(
              stage === "discover" ? "agents.discovering" : "agents.executing",
            )}
          </h3>
        </div>
      )}
      {stage === "configure" && discovery && (
        <div className="config-workbench">
          <aside className="catalog-rail">
            <p className="overline">{t("agents.commonCatalog")}</p>
            <strong>
              {t("agents.modelCount", { count: discovery.models.length })}
            </strong>
            <Catalog
              models={discovery.models}
              search={search}
              setSearch={setSearch}
              selected={[]}
              onSelect={() => undefined}
            />
          </aside>
          <div className="config-panels">
            {discovery.existing.drifted_agents.length > 0 && (
              <p className="drift-note" role="note">
                {t("agents.existingDrift", {
                  agents: discovery.existing.drifted_agents.join(", "),
                })}
              </p>
            )}
            {Object.entries(discovery.existing.unavailable_models).map(
              ([agent, models]) =>
                models?.length ? (
                  <p className="drift-note" role="note" key={agent}>
                    {t("agents.unavailableModels", {
                      agent,
                      models: models.join(", "),
                    })}
                  </p>
                ) : null,
            )}
            {selected.filter((agent) => sources[agent] !== "empty").length >
              0 && (
              <p className="source-note" role="note">
                {selected
                  .filter((agent) => sources[agent] !== "empty")
                  .map((agent) =>
                    t("agents.initializationSource", {
                      agent: agentNames[agent],
                      source: t(
                        sources[agent] === "existing"
                          ? "agents.source.existing"
                          : "agents.source.preset",
                      ),
                    }),
                  )
                  .join(" · ")}
              </p>
            )}
            {Object.entries(discovery.preset.unavailable_agents).map(
              ([agent, unavailable]) =>
                unavailable?.models.length ? (
                  <p className="drift-note" role="note" key={`preset-${agent}`}>
                    {t("agents.presetUnavailable", {
                      agent: agentNames[agent as AgentId],
                      models: unavailable.models.join(", "),
                    })}
                  </p>
                ) : null,
            )}
            {selected.includes("claude") && config.claude && (
              <section className="model-agent-panel">
                <h3>Claude Code</h3>
                <ClaudeSelectionFields
                  id="claude-primary"
                  selection={config.claude.primary}
                  models={discovery.models}
                  modelLabel={t("agents.primaryModel")}
                  onChange={(primary) =>
                    setConfig({
                      ...config,
                      claude: { ...config.claude!, primary },
                    })
                  }
                />
                <div className="typed-grid">
                  <OptionalNumber
                    label={t("agents.claudeContextWindow")}
                    value={config.claude.context_window}
                    onChange={(context_window) =>
                      setConfig({
                        ...config,
                        claude: { ...config.claude!, context_window },
                      })
                    }
                  />
                  <OptionalNumber
                    label={t("agents.claudeMaxOutputTokens")}
                    value={config.claude.max_output_tokens}
                    onChange={(max_output_tokens) =>
                      setConfig({
                        ...config,
                        claude: { ...config.claude!, max_output_tokens },
                      })
                    }
                  />
                </div>
                {roleNames.map((role) => (
                  <div className="role-row" key={role}>
                    <label>
                      <input
                        type="checkbox"
                        checked={"inherit_primary" in config.claude![role]}
                        onChange={(event) =>
                          setConfig({
                            ...config,
                            claude: {
                              ...config.claude!,
                              [role]: event.target.checked
                                ? { inherit_primary: true }
                                : { model: "" },
                            },
                          })
                        }
                      />
                      {t("agents.inheritPrimary", { role })}
                    </label>
                    {!("inherit_primary" in config.claude![role]) && (
                      <ClaudeSelectionFields
                        id={`claude-${role}`}
                        selection={config.claude![role] as ModelSelection}
                        models={discovery.models}
                        modelLabel={t("agents.roleModel", { role })}
                        onChange={(selection) =>
                          setConfig({
                            ...config,
                            claude: { ...config.claude!, [role]: selection },
                          })
                        }
                      />
                    )}
                  </div>
                ))}
                <div className="role-row">
                  <label>
                    <input
                      type="checkbox"
                      checked={Boolean(config.claude.fable)}
                      onChange={(event) => {
                        const claude = { ...config.claude! };
                        if (event.target.checked)
                          claude.fable = { inherit_primary: true };
                        else delete claude.fable;
                        setConfig({ ...config, claude });
                      }}
                    />
                    {t("agents.enableFable")}
                  </label>
                  {config.claude.fable && (
                    <div className="optional-role-editor">
                      <label>
                        <input
                          type="checkbox"
                          checked={"inherit_primary" in config.claude.fable}
                          onChange={(event) =>
                            setConfig({
                              ...config,
                              claude: {
                                ...config.claude!,
                                fable: event.target.checked
                                  ? { inherit_primary: true }
                                  : { model: "" },
                              },
                            })
                          }
                        />
                        {t("agents.inheritPrimary", { role: "fable" })}
                      </label>
                      {!("inherit_primary" in config.claude.fable) && (
                        <ClaudeSelectionFields
                          id="claude-fable"
                          selection={config.claude.fable}
                          models={discovery.models}
                          modelLabel={t("agents.roleModel", { role: "fable" })}
                          onChange={(fable) =>
                            setConfig({
                              ...config,
                              claude: { ...config.claude!, fable },
                            })
                          }
                        />
                      )}
                    </div>
                  )}
                </div>
                <ExtraEditor
                  agent="claude"
                  value={extras.claude}
                  onChange={(value) => setExtras({ ...extras, claude: value })}
                />
              </section>
            )}
            {selected.includes("opencode") && config.opencode && (
              <section className="model-agent-panel">
                <h3>opencode</h3>
                <div
                  className="catalog-list"
                  role="group"
                  aria-label={t("agents.opencodeModels")}
                >
                  {discovery.models
                    .filter((model) =>
                      model
                        .toLocaleLowerCase()
                        .includes(search.toLocaleLowerCase()),
                    )
                    .map((model) => (
                      <label
                        key={model}
                        className={
                          config.opencode!.models[model]
                            ? "catalog-model is-selected"
                            : "catalog-model"
                        }
                      >
                        <input
                          type="checkbox"
                          checked={Boolean(config.opencode!.models[model])}
                          onChange={() => {
                            const models = { ...config.opencode!.models };
                            if (models[model]) delete models[model];
                            else models[model] = {};
                            const default_model = models[
                              config.opencode!.default_model
                            ]
                              ? config.opencode!.default_model
                              : "";
                            setConfig({
                              ...config,
                              opencode: { models, default_model },
                            });
                          }}
                        />
                        <code>{safe(model)}</code>
                      </label>
                    ))}
                </div>
                <label className="option-field">
                  <span>{t("agents.defaultModel")}</span>
                  <AgentSelect
                    value={config.opencode.default_model}
                    onChange={(event) =>
                      setConfig({
                        ...config,
                        opencode: {
                          ...config.opencode!,
                          default_model: event.target.value,
                        },
                      })
                    }
                  >
                    <option value="">{t("agents.chooseModel")}</option>
                    {Object.keys(config.opencode.models).map((model) => (
                      <option key={model}>{model}</option>
                    ))}
                  </AgentSelect>
                </label>
                {Object.entries(config.opencode.models).map(
                  ([model, settings]) => (
                    <OpenCodeSettings
                      key={model}
                      model={model}
                      settings={settings}
                      onFieldError={(field, error) =>
                        setObjectFieldError(`${model}/${field}`, error)
                      }
                      update={(next) =>
                        setConfig({
                          ...config,
                          opencode: {
                            ...config.opencode!,
                            models: {
                              ...config.opencode!.models,
                              [model]: next,
                            },
                          },
                        })
                      }
                    />
                  ),
                )}
                <ExtraEditor
                  agent="opencode"
                  value={extras.opencode}
                  onChange={(value) =>
                    setExtras({ ...extras, opencode: value })
                  }
                />
              </section>
            )}
            {selected.includes("codex") && config.codex && (
              <section className="model-agent-panel">
                <h3>Codex</h3>
                <label className="option-field">
                  <span>{t("agents.activeModel")}</span>
                  <AgentSelect
                    value={config.codex.model}
                    onChange={(event) =>
                      setConfig({
                        ...config,
                        codex: { ...config.codex!, model: event.target.value },
                      })
                    }
                  >
                    <option value="">{t("agents.chooseModel")}</option>
                    {discovery.models.map((model) => (
                      <option key={model}>{model}</option>
                    ))}
                  </AgentSelect>
                </label>
                <div className="typed-grid">
                  <OptionalSelect
                    label={t("agents.reasoningEffort")}
                    value={config.codex.reasoning_effort}
                    values={[
                      "none",
                      "minimal",
                      "low",
                      "medium",
                      "high",
                      "xhigh",
                      "max",
                      "ultra",
                    ]}
                    onChange={(value) =>
                      setConfig({
                        ...config,
                        codex: { ...config.codex!, reasoning_effort: value },
                      })
                    }
                  />
                  <OptionalSelect
                    label={t("agents.reasoningSummary")}
                    value={config.codex.reasoning_summary}
                    values={["auto", "concise", "detailed", "none"]}
                    onChange={(value) =>
                      setConfig({
                        ...config,
                        codex: {
                          ...config.codex!,
                          reasoning_summary: value as
                            | "auto"
                            | "concise"
                            | "detailed"
                            | "none"
                            | undefined,
                        },
                      })
                    }
                  />
                  <OptionalSelect
                    label={t("agents.verbosity")}
                    value={config.codex.verbosity}
                    values={["low", "medium", "high"]}
                    onChange={(value) =>
                      setConfig({
                        ...config,
                        codex: {
                          ...config.codex!,
                          verbosity: value as
                            "low" | "medium" | "high" | undefined,
                        },
                      })
                    }
                  />
                  <OptionalNumber
                    label={t("agents.contextWindow")}
                    value={config.codex.context_window}
                    onChange={(value) =>
                      setConfig({
                        ...config,
                        codex: { ...config.codex!, context_window: value },
                      })
                    }
                  />
                  <OptionalNumber
                    label={t("agents.compactLimit")}
                    value={config.codex.auto_compact_token_limit}
                    onChange={(value) =>
                      setConfig({
                        ...config,
                        codex: {
                          ...config.codex!,
                          auto_compact_token_limit: value,
                        },
                      })
                    }
                  />
                  <OptionalSelect
                    label={t("agents.compactScope")}
                    value={
                      config.codex.extra?.model_auto_compact_token_limit_scope
                    }
                    values={["total", "body_after_prefix"]}
                    onChange={(value) =>
                      setConfig({
                        ...config,
                        codex: {
                          ...config.codex!,
                          extra: value
                            ? {
                                model_auto_compact_token_limit_scope: value as
                                  "total" | "body_after_prefix",
                              }
                            : undefined,
                        },
                      })
                    }
                  />
                </div>
                <ExtraEditor
                  agent="codex"
                  value={extras.codex}
                  onChange={(value) => setExtras({ ...extras, codex: value })}
                />
              </section>
            )}
            <div className="config-actions">
              <label className="text-button file-button">
                {t("agents.importConfig")}
                <input
                  type="file"
                  accept="application/json,.json"
                  onChange={(event) => void importConfig(event)}
                />
              </label>
              <button
                className="text-button"
                disabled={Boolean(invalid)}
                onClick={() => void exportConfig()}
              >
                {t("agents.exportConfig")}
              </button>
              <button
                className="control-button"
                disabled={Boolean(invalid) || busy}
                onClick={() => void loadPreview()}
              >
                {t("agents.generatePreview")}
              </button>
              <button className="text-button" onClick={() => void startOver()}>
                {t("agents.cancelDetection")}
              </button>
            </div>
            {invalid && (
              <p className="validation-path" role="alert">
                {invalid}
              </p>
            )}
          </div>
        </div>
      )}
      {stage === "preview" && preview && (
        <div className="preview-layout">
          <div className="preview-main">
            <section className="preview-agent">
              <div className="preview-agent__heading">
                <h3>{t("agents.fragments")}</h3>
              </div>
              {preview.fragments.map((fragment) => (
                <article
                  className="fragment-card"
                  key={`${fragment.agent}-${fragment.role}`}
                >
                  <strong>
                    {agentNames[fragment.agent]} / {safe(fragment.role)}
                  </strong>
                  <code>{safe(fragment.path)}</code>
                  <pre>{safe(fragment.content)}</pre>
                </article>
              ))}
            </section>
            <section className="preview-agent">
              <div className="preview-agent__heading">
                <h3>{t("agents.effects")}</h3>
              </div>
              <div className="preview-files">
                {preview.files.map((effect) => (
                  <FileEffect
                    key={`${effect.path}-${effect.role}`}
                    effect={effect}
                  />
                ))}
                {preview.state_change && (
                  <FileEffect effect={preview.state_change} />
                )}
              </div>
            </section>
          </div>
          <aside className="approval-rail">
            <p className="overline">{t("agents.approvalBoundary")}</p>
            <h3>{t("agents.reviewScope")}</h3>
            {preview.managed_collisions.map((collision) => (
              <p
                className="collision"
                key={`${collision.agent}-${collision.path}`}
              >
                {safe(collision.agent)} <code>{safe(collision.path)}</code>{" "}
                {safe(collision.action)}
              </p>
            ))}
            {preview.managed_config_drift && (
              <label className="approval-check">
                <input
                  type="checkbox"
                  checked={approveDrift}
                  onChange={(event) => setApproveDrift(event.target.checked)}
                />
                {t("agents.approveDrift")}
              </label>
            )}
            {preview.requires_codex_auth_approval && (
              <label className="approval-check">
                <input
                  type="checkbox"
                  checked={approveAuth}
                  onChange={(event) => setApproveAuth(event.target.checked)}
                />
                {t("agents.approveCodexAuth")}
              </label>
            )}
            <button
              className="control-button"
              disabled={
                (preview.managed_config_drift && !approveDrift) ||
                (preview.requires_codex_auth_approval && !approveAuth)
              }
              onClick={() => void write()}
            >
              {t("agents.write")}
            </button>
            <button
              className="text-button"
              onClick={() => setStage("configure")}
            >
              {t("agents.backToConfigure")}
            </button>
            <button className="text-button" onClick={() => void startOver()}>
              {t("agents.cancelDetection")}
            </button>
          </aside>
        </div>
      )}
      {stage === "result" && result && (
        <div className="agent-results">
          <div className="result-banner">
            <span>{t("agents.transactionComplete")}</span>
            <h3>{t("agents.resultHeading")}</h3>
          </div>
          <div className="result-grid">
            {result.agents.map((agent) => (
              <article key={agent.agent}>
                <div className="result-card__heading">
                  <h4>{agentNames[agent.agent]}</h4>
                  <span className={agent.success ? "result-ok" : "result-fail"}>
                    {agent.success ? t("agents.success") : t("agents.failure")}
                  </span>
                </div>
                {agent.changed?.map((path) => (
                  <code key={path}>{safe(path)}</code>
                ))}
                {agent.backups?.map((path) => (
                  <code key={path}>{safe(path)}</code>
                ))}
              </article>
            ))}
          </div>
          <button className="control-button" onClick={() => void startOver()}>
            {t("agents.finish")}
          </button>
        </div>
      )}
    </section>
  );
}

function OpenCodeSettings({
  model,
  settings,
  update,
  onFieldError,
}: {
  model: string;
  settings: OpenCodeModelConfig;
  update: (value: OpenCodeModelConfig) => void;
  onFieldError: (field: string, error?: string) => void;
}) {
  const { t } = useI18n();
  const flags = [
    "reasoning",
    "attachment",
    "tool_call",
    "temperature",
  ] as const;
  const modalities = ["text", "audio", "image", "video", "pdf"] as const;
  function toggleModality(
    kind: "input" | "output",
    modality: (typeof modalities)[number],
  ) {
    const current = settings.modalities?.[kind] ?? [];
    const next = current.includes(modality)
      ? current.filter((item) => item !== modality)
      : [...current, modality];
    const other =
      settings.modalities?.[kind === "input" ? "output" : "input"] ?? [];
    update({
      ...settings,
      modalities:
        next.length || other.length
          ? { ...settings.modalities, [kind]: next.length ? next : undefined }
          : undefined,
    });
  }
  return (
    <details className="model-settings">
      <summary>{safe(model)}</summary>
      <label className="option-field">
        <span>{t("agents.displayName")}</span>
        <input
          value={settings.name ?? ""}
          placeholder={t("agents.unset")}
          onChange={(event) =>
            update({ ...settings, name: event.target.value || undefined })
          }
        />
      </label>
      <div className="omission-grid">
        {flags.map((flag) => (
          <label key={flag}>
            <AgentSelect
              aria-label={`${model} ${flag}`}
              value={settings[flag] === undefined ? "" : String(settings[flag])}
              onChange={(event) =>
                update({
                  ...settings,
                  [flag]:
                    event.target.value === ""
                      ? undefined
                      : event.target.value === "true",
                })
              }
            >
              <option value="">{t("agents.unset")}</option>
              <option value="true">true</option>
              <option value="false">false</option>
            </AgentSelect>
            <span>{flag}</span>
          </label>
        ))}
      </div>
      <div className="typed-grid">
        <label className="option-field">
          <span>{t("agents.contextLimit")}</span>
          <input
            type="number"
            min="1"
            value={settings.limit?.context ?? ""}
            onChange={(event) => {
              const context = Number(event.target.value);
              update({
                ...settings,
                limit: context
                  ? {
                      context,
                      output: settings.limit?.output || 1,
                      input: settings.limit?.input,
                    }
                  : undefined,
              });
            }}
          />
        </label>
        <label className="option-field">
          <span>{t("agents.outputLimit")}</span>
          <input
            type="number"
            min="1"
            value={settings.limit?.output ?? ""}
            onChange={(event) => {
              const output = Number(event.target.value);
              update({
                ...settings,
                limit: output
                  ? {
                      context: settings.limit?.context || output,
                      output,
                      input: settings.limit?.input,
                    }
                  : undefined,
              });
            }}
          />
        </label>
      </div>
      <div className="modality-grid">
        {(["input", "output"] as const).map((kind) => (
          <fieldset key={kind}>
            <legend>{t(`agents.modalities.${kind}`)}</legend>
            {modalities.map((modality) => (
              <label key={modality}>
                <input
                  type="checkbox"
                  checked={
                    settings.modalities?.[kind]?.includes(modality) ?? false
                  }
                  onChange={() => toggleModality(kind, modality)}
                />
                {modality}
              </label>
            ))}
          </fieldset>
        ))}
      </div>
      <OptionalSelect
        label={t("agents.interleaved")}
        value={
          settings.interleaved === true ? "true" : settings.interleaved?.field
        }
        values={["true", "reasoning", "reasoning_content", "reasoning_details"]}
        onChange={(value) =>
          update({
            ...settings,
            interleaved:
              value === "true"
                ? true
                : value
                  ? {
                      field: value as
                        "reasoning" | "reasoning_content" | "reasoning_details",
                    }
                  : undefined,
          })
        }
      />
      <div className="typed-grid">
        <ObjectField
          label={t("agents.optionsJson")}
          value={settings.options}
          onChange={(value) => update({ ...settings, options: value })}
          onErrorChange={(error) => onFieldError("options", error)}
        />
        <ObjectField
          label={t("agents.variantsJson")}
          value={settings.variants}
          onChange={(value) =>
            update({
              ...settings,
              variants: value as Record<string, JsonObject> | undefined,
            })
          }
          onErrorChange={(error) => onFieldError("variants", error)}
        />
        <ObjectField
          label={t("agents.modelExtraJson")}
          value={settings.extra}
          onChange={(value) => update({ ...settings, extra: value })}
          onErrorChange={(error) => onFieldError("extra", error)}
        />
      </div>
    </details>
  );
}
