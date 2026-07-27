import {
  useEffect,
  useEffectEvent,
  useRef,
  useState,
  type ComponentPropsWithoutRef,
} from "react";

import type { OverviewIssue } from "./AgentOverview";
import { agentNames, type AgentTarget } from "./agentPresentation";
import { useI18n, type Translator } from "./i18n";
import {
  initializeAgentConfig,
  sanitizeSensitiveText,
  type AgentDetection,
  type AgentFileEffect,
  type AgentId,
  type AgentModes,
  type AgentModelsResult,
  type AgentPreview,
  type AgentWriteResult,
  type DesktopApi,
  type InitializationSource,
  type JsonObject,
  type ModelConfig,
  type ModelSelection,
  type OpenCodeModelConfig,
} from "./ipc";

interface AgentWorkflowProps {
  api: DesktopApi;
  target: AgentTarget;
  discovery: AgentModelsResult;
  onBack(): void;
  onFlowConsumed(): void;
  onReturnToOverview(issue?: OverviewIssue): void;
  refreshDetection(): Promise<AgentDetection>;
}

type WorkflowStage = "configure" | "preview" | "write" | "result";
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

function retryIssue(code: string, target: AgentTarget): OverviewIssue {
  return { kind: "retry", code: code || "UNKNOWN", target };
}

function writeIssue(code: string, target: AgentTarget): OverviewIssue {
  if (
    [
      "CREDENTIAL_NOT_FOUND",
      "CREDENTIAL_INVALID",
      "CREDENTIAL_IO_ERROR",
      "CREDENTIAL_LOCK_TIMEOUT",
    ].includes(code)
  ) {
    return { kind: "credential", code };
  }
  if (code === "MODEL_AUTH_FAILED") return { kind: "auth", code };
  return retryIssue(code, target);
}

function targetIsReusable(detection: AgentDetection, target: AgentTarget) {
  const state = detection.agents.find((agent) => agent.agent === target.agent);
  return target.mode === "rebuild"
    ? Boolean(state?.invalid && state.recovery.eligible)
    : Boolean(state?.writable && !state.invalid);
}

function modelConfigErrorMessage(error: unknown, t: Translator) {
  const details =
    typeof error === "object" && error !== null && "details" in error
      ? (error as { details?: unknown }).details
      : undefined;
  const path =
    typeof details === "object" &&
    details !== null &&
    "path" in details &&
    typeof (details as { path?: unknown }).path === "string"
      ? (details as { path: string }).path
      : "";
  const rule =
    typeof details === "object" &&
    details !== null &&
    "rule" in details &&
    typeof (details as { rule?: unknown }).rule === "string"
      ? (details as { rule: string }).rule
      : "";

  if (rule === "catalog_model" || rule === "catalog")
    return t("agents.error.config.catalogModel");
  if (rule === "base_model" || rule === "model_id")
    return t("agents.error.config.baseModel");
  if (rule === "non_empty_name" || rule === "name")
    return t("agents.error.config.name");
  if (rule === "context_conflict")
    return t("agents.error.config.contextConflict");
  if (rule === "integer_relationship") {
    return path === "/claude/max_output_tokens"
      ? t("agents.error.config.outputLimit")
      : t("agents.error.config.integerRelationship");
  }
  if (rule === "positive_integer")
    return path.endsWith("/context_window")
      ? t("agents.error.config.contextWindow")
      : t("agents.error.config.positiveInteger");
  if (rule === "allowlist" || rule === "protected_path")
    return t("agents.error.config.extra");
  return t("agents.error.config.fallback");
}

function previewErrorMessage(code: string, t: Translator) {
  const key =
    code === "CONFIG_INVALID"
      ? "agents.error.preview.configInvalid"
      : code === "CONFIG_NOT_WRITABLE"
        ? "agents.error.preview.notWritable"
        : code === "AGENT_NOT_FOUND"
          ? "agents.error.preview.agentNotFound"
          : code === "MODEL_STATE_INVALID"
            ? "agents.error.preview.modelState"
            : code === "AGENT_OPERATION_BUSY"
              ? "agents.error.preview.busy"
              : code === "OPERATION_TIMEOUT"
                ? "agents.error.preview.timeout"
                : [
                      "MANAGER_FAILED",
                      "SIDECAR_MISSING",
                      "SIDECAR_INVALID",
                      "INVALID_RESPONSE",
                    ].includes(code)
                  ? "agents.error.preview.manager"
                  : "agents.error.preview.unknown";
  return t(key, { code: code || "UNKNOWN" });
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
  onChange(selection: ModelSelection): void;
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

function OptionalSelect({
  label,
  value,
  values,
  onChange,
}: {
  label: string;
  value?: string;
  values: string[];
  onChange(value?: string): void;
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
          <option key={item}>{item}</option>
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
  onChange(value?: number): void;
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
  onChange(value?: JsonObject): void;
  onErrorChange(error?: string): void;
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

function ExtraEditor({
  agent,
  value,
  onChange,
}: {
  agent: AgentId;
  value: string;
  onChange(value: string): void;
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

function Catalog({
  models,
  search,
  setSearch,
}: {
  models: string[];
  search: string;
  setSearch(value: string): void;
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
        role="radiogroup"
        aria-label={t("agents.catalogLabel")}
      >
        {visible.map((model) => (
          <div className="catalog-model" key={model}>
            <code>{safe(model)}</code>
          </div>
        ))}
        {!visible.length && <p>{t("agents.catalogEmptySearch")}</p>}
      </div>
    </div>
  );
}

function FileEffect({ effect }: { effect: AgentFileEffect }) {
  const { t } = useI18n();
  const operation =
    effect.operation === "create"
      ? t("agents.operation.create")
      : effect.operation === "replace"
        ? t("agents.operation.replace")
        : effect.operation === "preserve"
          ? t("agents.operation.preserve")
          : safe(effect.operation);
  return (
    <article
      className={`effect-card${effect.mode === "rebuild" ? " effect-card--rebuild" : ""}`}
    >
      <div className="effect-card__badges">
        <span>{operation}</span>
        {effect.mode && <span>{t(`agents.mode.${effect.mode}`)}</span>}
        {effect.backup_sensitive && (
          <span className="effect-card__sensitive">
            {t("agents.sensitiveBackup")}
          </span>
        )}
      </div>
      <dl className="effect-card__details">
        {effect.agent && (
          <div>
            <dt>{t("agents.effect.agent")}</dt>
            <dd>{agentNames[effect.agent]}</dd>
          </div>
        )}
        <div>
          <dt>{t("agents.effect.role")}</dt>
          <dd>{safe(effect.role)}</dd>
        </div>
      </dl>
      <span className="effect-card__label">{t("agents.effect.path")}</span>
      <code>{safe(effect.path)}</code>
      {effect.backup_required && effect.backup_pattern && (
        <>
          <span className="effect-card__label">
            {t("agents.effect.backupPattern")}
          </span>
          <code className="backup-path">{safe(effect.backup_pattern)}</code>
        </>
      )}
      {effect.backup_path && (
        <code className="backup-path">{safe(effect.backup_path)}</code>
      )}
      {effect.preserves && effect.preserves.length > 0 && (
        <p>
          <strong>{t("agents.effect.preserves")}</strong>{" "}
          {effect.preserves.map(safe).join(", ")}
        </p>
      )}
      {effect.warning && (
        <p className="effect-card__warning">
          <strong>{t("agents.effect.warning")}</strong> {safe(effect.warning)}
        </p>
      )}
    </article>
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
  update(value: OpenCodeModelConfig): void;
  onFieldError(field: string, error?: string): void;
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
        <OptionalNumber
          label={t("agents.contextLimit")}
          value={settings.limit?.context}
          onChange={(context) =>
            update({
              ...settings,
              limit: context
                ? {
                    context,
                    output: settings.limit?.output || 1,
                    input: settings.limit?.input,
                  }
                : undefined,
            })
          }
        />
        <OptionalNumber
          label={t("agents.outputLimit")}
          value={settings.limit?.output}
          onChange={(output) =>
            update({
              ...settings,
              limit: output
                ? {
                    context: settings.limit?.context || output,
                    output,
                    input: settings.limit?.input,
                  }
                : undefined,
            })
          }
        />
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

function validateSingleTargetConfig(config: ModelConfig, target: AgentId) {
  return (
    config[target] !== undefined &&
    (["claude", "opencode", "codex"] as AgentId[]).every(
      (agent) => agent === target || config[agent] === undefined,
    )
  );
}

function validateRebuildPreview(
  preview: AgentPreview | null,
  target: AgentTarget,
) {
  if (
    !preview ||
    !validateSingleTargetConfig(preview.model_config, target.agent)
  )
    return false;
  if (preview.fragments.some((fragment) => fragment.agent !== target.agent))
    return false;
  if (preview.drifted_agents.some((agent) => agent !== target.agent))
    return false;
  if (
    preview.managed_collisions.some(
      (collision) => collision.agent !== target.agent,
    )
  )
    return false;
  if (
    preview.files.some(
      (effect) => effect.agent && effect.agent !== target.agent,
    )
  )
    return false;
  const rebuildEffects = preview.files.filter(
    (effect) => effect.mode === "rebuild",
  );
  const unique = new Set(
    rebuildEffects.map((effect) => `${effect.agent}\0${effect.path}`),
  );
  if (unique.size !== rebuildEffects.length) return false;
  if (target.mode === "merge") return rebuildEffects.length === 0;
  return (
    rebuildEffects.length > 0 &&
    rebuildEffects.every((effect) => effect.agent === target.agent) &&
    !preview.files.some(
      (effect) => effect.agent === target.agent && effect.mode === "merge",
    )
  );
}

function validateWriteResult(
  result: unknown,
  target: AgentId,
): result is AgentWriteResult {
  if (!result || typeof result !== "object") return false;
  const statuses = (result as { agents?: unknown }).agents;
  if (!Array.isArray(statuses) || statuses.length !== 1) return false;
  const status = statuses[0];
  return (
    Boolean(status) &&
    typeof status === "object" &&
    (status as { agent?: unknown }).agent === target &&
    typeof (status as { success?: unknown }).success === "boolean"
  );
}

export function AgentWorkflow({
  api,
  target,
  discovery,
  onBack,
  onFlowConsumed,
  onReturnToOverview,
  refreshDetection,
}: AgentWorkflowProps) {
  const { t } = useI18n();
  const agents: AgentId[] = [target.agent];
  const modes: AgentModes = { [target.agent]: target.mode };
  const initialized = initializeAgentConfig(
    agents,
    discovery.existing.model_config,
    discovery.preset.model_config,
  );
  const [stage, setStage] = useState<WorkflowStage>("configure");
  const [config, setConfig] = useState<ModelConfig>(initialized.config);
  const [sources, setSources] = useState<Record<AgentId, InitializationSource>>(
    initialized.sources,
  );
  const [preview, setPreview] = useState<AgentPreview | null>(null);
  const [result, setResult] = useState<AgentWriteResult | null>(null);
  const [search, setSearch] = useState("");
  const [extra, setExtra] = useState("");
  const [objectFieldErrors, setObjectFieldErrors] = useState<
    Record<string, string>
  >({});
  const [approveDrift, setApproveDrift] = useState(false);
  const [approveAuth, setApproveAuth] = useState(false);
  const [rebuildDialogOpen, setRebuildDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const writeInFlightRef = useRef(false);
  const cancelRebuildRef = useRef<HTMLButtonElement>(null);
  const confirmRebuildRef = useRef<HTMLButtonElement>(null);
  const rebuildTriggerRef = useRef<HTMLButtonElement>(null);

  function resetApprovals() {
    setApproveDrift(false);
    setApproveAuth(false);
    setRebuildDialogOpen(false);
  }

  useEffect(() => {
    if (!rebuildDialogOpen) return;
    cancelRebuildRef.current?.focus();
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !writeInFlightRef.current) {
        rebuildTriggerRef.current?.focus();
        setRebuildDialogOpen(false);
      }
    }
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [rebuildDialogOpen]);

  function closeRebuildDialog() {
    rebuildTriggerRef.current?.focus();
    setRebuildDialogOpen(false);
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

  function configError() {
    if (!validateSingleTargetConfig(config, target.agent))
      return "/: single_agent_required";
    if (target.agent === "claude" && config.claude) {
      if (!config.claude.primary.model)
        return "/claude/primary/model: required";
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
      target.agent === "opencode" &&
      config.opencode &&
      (!Object.keys(config.opencode.models).length ||
        !config.opencode.default_model)
    )
      return "/opencode/default_model: required";
    if (target.agent === "codex" && config.codex && !config.codex.model)
      return "/codex/model: required";
    const parsed = parseExtra(extra);
    if (parsed.error) return `/${target.agent}/extra${parsed.error}`;
    const objectError = Object.entries(objectFieldErrors)[0];
    if (objectError)
      return `/opencode/models/${objectError[0]}: ${objectError[1]}`;
    return "";
  }

  function finalConfig() {
    const value = structuredClone(config);
    const parsed = parseExtra(extra).value;
    if (parsed && Object.keys(parsed).length) {
      if (target.agent === "claude" && value.claude)
        value.claude.extra = Object.fromEntries(
          Object.entries(parsed).map(([key, child]) => [key, String(child)]),
        );
      if (target.agent === "codex" && value.codex)
        value.codex.extra = parsed as NonNullable<typeof value.codex.extra>;
    }
    return value;
  }

  async function loadPreview() {
    const invalid = configError();
    if (invalid) {
      setMessage(invalid);
      return;
    }
    setBusy(true);
    setMessage("");
    try {
      const value = await api.previewAgents(
        agents,
        discovery.flow_id,
        discovery.catalog_token,
        finalConfig(),
        modes,
      );
      if (!validateSingleTargetConfig(value.model_config, target.agent)) {
        onReturnToOverview(retryIssue("MODEL_RESPONSE_INVALID", target));
        return;
      }
      if (!value.revision_token.trim()) {
        onReturnToOverview(retryIssue("MODEL_RESPONSE_INVALID", target));
        return;
      }
      resetApprovals();
      setPreview(value);
      setConfig(value.model_config);
      setStage("preview");
    } catch (error) {
      const code = errorCode(error);
      if (code === "MODEL_CATALOG_STALE" || code === "MODEL_FLOW_EXPIRED") {
        onReturnToOverview(retryIssue(code, target));
      } else if (code === "MODEL_CONFIG_INVALID") {
        setMessage(
          t("agents.error.config", {
            detail: modelConfigErrorMessage(error, t),
          }),
        );
      } else {
        setMessage(previewErrorMessage(code, t));
      }
    } finally {
      setBusy(false);
    }
  }

  async function recoverStalePreview() {
    setPreview(null);
    resetApprovals();
    try {
      const detection = await refreshDetection();
      if (targetIsReusable(detection, target)) {
        setStage("configure");
        setMessage(t("agents.error.previewStale"));
      } else {
        onReturnToOverview(retryIssue("PREVIEW_STALE", target));
      }
    } catch {
      onReturnToOverview(retryIssue("PREVIEW_STALE", target));
    }
  }

  async function write(approveRebuild: AgentId[]) {
    if (
      !preview ||
      writeInFlightRef.current ||
      !validateRebuildPreview(preview, target)
    )
      return;
    writeInFlightRef.current = true;
    setBusy(true);
    setStage("write");
    setMessage("");
    try {
      const value = await api.writeAgents(
        agents,
        discovery.flow_id,
        discovery.catalog_token,
        config,
        preview.revision_token,
        approveDrift,
        approveAuth,
        approveRebuild,
      );
      if (!validateWriteResult(value, target.agent)) {
        onReturnToOverview(retryIssue("INVALID_RESPONSE", target));
        return;
      }
      if (value.agents[0].success) onFlowConsumed();
      setResult(value);
      setStage("result");
    } catch (error) {
      const code = errorCode(error);
      if (code === "PREVIEW_STALE") {
        await recoverStalePreview();
      } else {
        if (code === "ROLLBACK_FAILED") {
          try {
            const detection = await refreshDetection();
            if (targetIsReusable(detection, target))
              onReturnToOverview(retryIssue(code, target));
            else onReturnToOverview();
          } catch {
            onReturnToOverview({ kind: "detect", code });
          }
        } else {
          onReturnToOverview(writeIssue(code, target));
        }
      }
    } finally {
      writeInFlightRef.current = false;
      setBusy(false);
    }
  }

  async function importConfig(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    if (
      file.size > 2 * 1024 * 1024 ||
      !file.name.toLowerCase().endsWith(".json")
    ) {
      setMessage(t("agents.error.import"));
      return;
    }
    try {
      const value = await api.importAgentModelConfig(
        await file.text(),
        agents,
        discovery.flow_id,
      );
      if (!validateSingleTargetConfig(value, target.agent))
        throw new Error("invalid target");
      setConfig(value);
      setExtra("");
      setObjectFieldErrors({});
      setSources({ claude: "empty", opencode: "empty", codex: "empty" });
      setMessage(t("agents.imported"));
    } catch {
      setMessage(t("agents.error.import"));
    }
  }

  async function exportConfig() {
    if (configError()) return;
    try {
      const content = await api.exportAgentModelConfig(
        finalConfig(),
        agents,
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

  async function finish() {
    setBusy(true);
    try {
      await refreshDetection();
      onReturnToOverview();
    } catch {
      onReturnToOverview({ kind: "detect", code: "AGENT_DETECT_FAILED" });
    } finally {
      setBusy(false);
    }
  }

  const invalid = configError();
  const rebuildPreviewValid = validateRebuildPreview(preview, target);
  const targetResult = result?.agents.find(
    (agent) => agent.agent === target.agent,
  );
  const showInstallLater =
    !target.installedAtEntry && targetResult?.success === true;
  return (
    <section className="agents-workbench" aria-labelledby="agents-heading">
      <header className="agents-workbench__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 id="agents-heading">{agentNames[target.agent]}</h2>
        </div>
      </header>
      {message && (
        <p className="agent-alert" role="alert">
          {message}
        </p>
      )}
      {stage === "write" && (
        <div className="processing-stage" role="status">
          <span className="instrument__dial">TX</span>
          <h3>{t("agents.executing")}</h3>
        </div>
      )}
      {stage === "configure" && (
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
            />
          </aside>
          <div className="config-panels">
            {discovery.existing.drifted_agents.includes(target.agent) && (
              <p className="drift-note" role="note">
                {t("agents.existingDrift", { agents: target.agent })}
              </p>
            )}
            {(discovery.existing.unavailable_models[target.agent]?.length ??
              0) > 0 && (
              <p className="drift-note" role="note">
                {t("agents.unavailableModels", {
                  agent: target.agent,
                  models:
                    discovery.existing.unavailable_models[target.agent]!.map(
                      safe,
                    ).join(", "),
                })}
              </p>
            )}
            {sources[target.agent] !== "empty" && (
              <p className="source-note" role="note">
                {t("agents.initializationSource", {
                  agent: agentNames[target.agent],
                  source: t(
                    sources[target.agent] === "existing"
                      ? "agents.source.existing"
                      : "agents.source.preset",
                  ),
                })}
              </p>
            )}
            {(discovery.preset.unavailable_agents[target.agent]?.models
              .length ?? 0) > 0 && (
              <p className="drift-note" role="note">
                {t("agents.presetUnavailable", {
                  agent: agentNames[target.agent],
                  models:
                    discovery.preset.unavailable_agents[
                      target.agent
                    ]!.models.map(safe).join(", "),
                })}
              </p>
            )}
            {target.agent === "claude" && config.claude && (
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
                <ExtraEditor agent="claude" value={extra} onChange={setExtra} />
              </section>
            )}
            {target.agent === "opencode" && config.opencode && (
              <section className="model-agent-panel">
                <h3>OpenCode</h3>
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
                  value={extra}
                  onChange={setExtra}
                />
              </section>
            )}
            {target.agent === "codex" && config.codex && (
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
                    onChange={(reasoning_effort) =>
                      setConfig({
                        ...config,
                        codex: { ...config.codex!, reasoning_effort },
                      })
                    }
                  />
                  <OptionalSelect
                    label={t("agents.reasoningSummary")}
                    value={config.codex.reasoning_summary}
                    values={["auto", "concise", "detailed", "none"]}
                    onChange={(reasoning_summary) =>
                      setConfig({
                        ...config,
                        codex: {
                          ...config.codex!,
                          reasoning_summary: reasoning_summary as
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
                    onChange={(verbosity) =>
                      setConfig({
                        ...config,
                        codex: {
                          ...config.codex!,
                          verbosity: verbosity as
                            "low" | "medium" | "high" | undefined,
                        },
                      })
                    }
                  />
                  <OptionalNumber
                    label={t("agents.contextWindow")}
                    value={config.codex.context_window}
                    onChange={(context_window) =>
                      setConfig({
                        ...config,
                        codex: { ...config.codex!, context_window },
                      })
                    }
                  />
                  <OptionalNumber
                    label={t("agents.compactLimit")}
                    value={config.codex.auto_compact_token_limit}
                    onChange={(auto_compact_token_limit) =>
                      setConfig({
                        ...config,
                        codex: { ...config.codex!, auto_compact_token_limit },
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
                <ExtraEditor agent="codex" value={extra} onChange={setExtra} />
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
              <button className="text-button" onClick={onBack}>
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
              ref={rebuildTriggerRef}
              className="control-button"
              disabled={
                (preview.managed_config_drift && !approveDrift) ||
                (preview.requires_codex_auth_approval && !approveAuth) ||
                !rebuildPreviewValid
              }
              onClick={() => {
                if (!rebuildPreviewValid) return;
                if (target.mode === "rebuild") setRebuildDialogOpen(true);
                else void write([]);
              }}
            >
              {t("agents.write")}
            </button>
            <button
              className="text-button"
              onClick={() => setStage("configure")}
            >
              {t("agents.backToConfigure")}
            </button>
            <button className="text-button" onClick={onBack}>
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
          {showInstallLater && (
            <p className="result-install-note" role="note">
              {t("agents.result.installLater", {
                agent: agentNames[target.agent],
              })}
            </p>
          )}
          <div className="result-grid">
            {result.agents.map((agent) => (
              <article key={agent.agent}>
                <div className="result-card__heading">
                  <h4>{agentNames[agent.agent]}</h4>
                  {target.mode === "rebuild" && (
                    <span className="result-mode">
                      {t("agents.mode.rebuild")}
                    </span>
                  )}
                  <span className={agent.success ? "result-ok" : "result-fail"}>
                    {agent.success ? t("agents.success") : t("agents.failure")}
                  </span>
                </div>
                {agent.changed?.map((path) => (
                  <div className="result-path" key={path}>
                    <span>{t("agents.changed")}</span>
                    <code>{safe(path)}</code>
                  </div>
                ))}
                {agent.backups?.map((path) => (
                  <div className="result-path result-path--backup" key={path}>
                    <span>{t("agents.backups")}</span>
                    <code>{safe(path)}</code>
                  </div>
                ))}
              </article>
            ))}
          </div>
          <button
            className="control-button"
            disabled={busy}
            onClick={() => void finish()}
          >
            {t("agents.finish")}
          </button>
        </div>
      )}
      {rebuildDialogOpen && rebuildPreviewValid && (
        <div
          className="dialog-backdrop"
          onMouseDown={(event) => {
            if (
              event.target === event.currentTarget &&
              !writeInFlightRef.current
            )
              closeRebuildDialog();
          }}
        >
          <section
            className="danger-dialog rebuild-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rebuild-dialog-title"
            aria-describedby="rebuild-dialog-warning"
            onKeyDown={(event) => {
              if (event.key !== "Tab") return;
              if (
                event.shiftKey &&
                document.activeElement === cancelRebuildRef.current
              ) {
                event.preventDefault();
                confirmRebuildRef.current?.focus();
              } else if (
                !event.shiftKey &&
                document.activeElement === confirmRebuildRef.current
              ) {
                event.preventDefault();
                cancelRebuildRef.current?.focus();
              }
            }}
          >
            <p className="overline">{t("agents.rebuildConfirm.overline")}</p>
            <h2 id="rebuild-dialog-title">
              {t("agents.rebuildConfirm.title")}
            </h2>
            <p>{t("agents.rebuildConfirm.description")}</p>
            <ul className="rebuild-dialog__agents">
              <li>{agentNames[target.agent]}</li>
            </ul>
            <p id="rebuild-dialog-warning" className="danger-dialog__warning">
              {t("agents.recovery.warning")}
            </p>
            <div className="danger-dialog__actions">
              <button
                ref={cancelRebuildRef}
                type="button"
                className="text-button"
                disabled={busy}
                onClick={closeRebuildDialog}
              >
                {t("agents.rebuildConfirm.cancel")}
              </button>
              <button
                ref={confirmRebuildRef}
                type="button"
                className="control-button control-button--danger"
                disabled={busy}
                onClick={() => {
                  setRebuildDialogOpen(false);
                  void write([target.agent]);
                }}
              >
                {t("agents.rebuildConfirm.confirm")}
              </button>
            </div>
          </section>
        </div>
      )}
    </section>
  );
}
