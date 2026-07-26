import {
  useEffect,
  useEffectEvent,
  useRef,
  useState,
  type ComponentPropsWithoutRef,
} from "react";

import { useI18n, type Translator } from "./i18n";
import {
  sanitizeSensitiveText,
  initializeAgentConfig,
  type AgentDetection,
  type AgentFileEffect,
  type AgentId,
  type AgentMode,
  type AgentModes,
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
  opencode: "OpenCode",
  codex: "Codex",
};
type SelectedModes = Partial<Record<AgentId, AgentMode>>;
const roleNames = ["haiku", "sonnet", "opus"] as const;

function AgentLogo({ agent }: { agent: AgentId }) {
  if (agent === "claude") {
    return (
      <svg
        aria-hidden="true"
        className="agent-logo agent-logo--claude"
        viewBox="0 0 24 24"
        focusable="false"
      >
        <path d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z" />
      </svg>
    );
  }
  if (agent === "opencode") {
    return (
      <svg
        aria-hidden="true"
        className="agent-logo agent-logo--opencode"
        viewBox="0 0 24 24"
        focusable="false"
      >
        <path d="M22 24H2V0h20zM17 4.8H7v14.4h10z" />
      </svg>
    );
  }
  return (
    <svg
      aria-hidden="true"
      className="agent-logo agent-logo--codex"
      viewBox="0 0 24 24"
      focusable="false"
    >
      <path d="M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z" />
    </svg>
  );
}

function isAgentId(value: unknown): value is AgentId {
  return agentOrder.some((agent) => agent === value);
}

function validateRebuildPreview(
  preview: AgentPreview | null,
  selected: AgentId[],
) {
  if (!preview) return null;
  const previewAgents = new Set<AgentId>();
  const previewFiles = new Set<string>();
  for (const effect of preview.files) {
    if (effect.mode !== "rebuild") continue;
    if (!isAgentId(effect.agent)) return null;
    const fileKey = `${effect.agent}\0${effect.path}`;
    if (previewFiles.has(fileKey)) return null;
    previewFiles.add(fileKey);
    previewAgents.add(effect.agent);
  }
  if (
    previewAgents.size !== selected.length ||
    selected.some((agent) => !previewAgents.has(agent))
  ) {
    return null;
  }
  return selected;
}

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
    if (path === "/claude/max_output_tokens")
      return t("agents.error.config.outputLimit");
    return t("agents.error.config.integerRelationship");
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
                : code === "MANAGER_FAILED" ||
                    code === "SIDECAR_MISSING" ||
                    code === "SIDECAR_INVALID" ||
                    code === "INVALID_RESPONSE"
                  ? "agents.error.preview.manager"
                  : "agents.error.preview.unknown";
  return t(key, { code: code || "UNKNOWN" });
}
function selectable(agent: AgentState) {
  return agent.detected && agent.writable && !agent.invalid;
}
function initialSelection(detection: AgentDetection) {
  return Object.fromEntries(
    detection.agents.filter(selectable).map((agent) => [agent.agent, "merge"]),
  ) as SelectedModes;
}
function staleSelectionIsReusable(
  detection: AgentDetection,
  selectedModes: SelectedModes,
) {
  const states = new Map(detection.agents.map((agent) => [agent.agent, agent]));
  return (
    agentOrder.some((agent) => selectedModes[agent]) &&
    agentOrder.every((agent) => {
      const mode = selectedModes[agent];
      if (!mode) return true;
      const state = states.get(agent);
      return mode === "rebuild"
        ? Boolean(state?.invalid && state.recovery?.eligible)
        : Boolean(state && selectable(state));
    })
  );
}
function recoveryReasons(agent: AgentState) {
  return [
    ...(agent.recovery?.reasons ?? []),
    ...(agent.recovery?.files.flatMap((file) => file.reasons ?? []) ?? []),
  ].filter((reason, index, reasons) => reasons.indexOf(reason) === index);
}
function recoveryReason(reason: string, t: Translator) {
  switch (reason) {
    case "syntax_invalid":
      return t("agents.recovery.reason.syntaxInvalid");
    case "unsupported_structure":
      return t("agents.recovery.reason.unsupportedStructure");
    case "unreadable":
      return t("agents.recovery.reason.unreadable");
    case "oversized":
      return t("agents.recovery.reason.oversized");
    case "non_regular":
      return t("agents.recovery.reason.nonRegular");
    case "linked":
      return t("agents.recovery.reason.linked");
    case "not_writable":
      return t("agents.recovery.reason.notWritable");
    case "parent_unavailable":
      return t("agents.recovery.reason.parentUnavailable");
    case "transaction_recovery_pending":
      return t("agents.recovery.reason.transactionPending");
    case "writes_disabled":
      return t("agents.recovery.reason.writesDisabled");
    default:
      return t("agents.recovery.reason.unknown");
  }
}
function detectionState(agent: AgentState | undefined, t: Translator) {
  if (!agent?.detected) return t("agents.detection.absent");
  if (agent.invalid) return t("agents.detection.invalid");
  if (!agent.writable) return t("agents.detection.readonly");
  if (agent.configured) return t("agents.detection.configured");
  if (!agent.exists) return t("agents.detection.create");
  return t("agents.detection.ready");
}
function detectionGuidance(agent: AgentState | undefined, t: Translator) {
  if (!agent?.detected) return t("agents.guidance.absent");
  if (agent.invalid)
    return t(
      agent.recovery?.eligible
        ? "agents.recovery.guidance.eligible"
        : "agents.recovery.guidance.ineligible",
    );
  if (!agent.writable) return t("agents.guidance.readonly");
  if (agent.configured) return t("agents.guidance.configured");
  return t("agents.guidance.ready");
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

export function AgentPage({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [detection, setDetection] = useState<AgentDetection | null>(null);
  const [selectedModes, setSelectedModes] = useState<SelectedModes>({});
  const [stage, setStage] = useState<Stage>("select");
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
  const [rebuildDialogOpen, setRebuildDialogOpen] = useState(false);
  const [busy, setBusy] = useState(true);
  const [message, setMessage] = useState("");
  const flowRef = useRef("");
  const writeInFlightRef = useRef(false);
  const cancelRebuildRef = useRef<HTMLButtonElement>(null);
  const confirmRebuildRef = useRef<HTMLButtonElement>(null);
  const rebuildTriggerRef = useRef<HTMLButtonElement>(null);
  const reportDetectionFailure = useEffectEvent(() =>
    setMessage(t("agents.error.detect")),
  );
  const selected = agentOrder.filter((agent) => selectedModes[agent]);
  const modes = Object.fromEntries(
    selected.map((agent) => [agent, selectedModes[agent]]),
  ) as AgentModes;

  async function destroyFlow() {
    const flow = flowRef.current;
    flowRef.current = "";
    if (flow) await api.destroyAgentModelFlow(flow).catch(() => undefined);
  }
  function resetApprovals() {
    setApproveDrift(false);
    setApproveAuth(false);
    setRebuildDialogOpen(false);
  }
  function clearFlowState(target: Stage) {
    flowRef.current = "";
    setDiscovery(null);
    setConfig({ version: 1 });
    setSources({ claude: "empty", opencode: "empty", codex: "empty" });
    setPreview(null);
    setResult(null);
    setExtras({ claude: "", opencode: "", codex: "" });
    setObjectFieldErrors({});
    setSearch("");
    resetApprovals();
    setStage(target);
  }
  async function refreshDetection(target: Stage = "select") {
    await destroyFlow();
    clearFlowState(target);
    setDetection(null);
    setSelectedModes({});
    const value = await api.detectAgents();
    setDetection(value);
    setSelectedModes(initialSelection(value));
  }
  async function recoverStalePreview() {
    setPreview(null);
    resetApprovals();
    try {
      const value = await api.detectAgents();
      setDetection(value);
      if (staleSelectionIsReusable(value, selectedModes)) {
        setStage("configure");
        setMessage(t("agents.error.previewStale"));
        return;
      }
      await destroyFlow();
      clearFlowState("select");
      setSelectedModes(initialSelection(value));
      setMessage(t("agents.error.previewStale"));
    } catch {
      await destroyFlow();
      clearFlowState("select");
      setDetection(null);
      setSelectedModes({});
      setMessage(t("agents.error.detect"));
    }
  }
  useEffect(() => {
    let active = true;
    void api
      .detectAgents()
      .then((value) => {
        if (active) {
          setDetection(value);
          setSelectedModes(initialSelection(value));
        }
      })
      .catch(() => active && reportDetectionFailure())
      .finally(() => active && setBusy(false));
    return () => {
      active = false;
      const flow = flowRef.current;
      flowRef.current = "";
      if (flow) void api.destroyAgentModelFlow(flow);
    };
  }, [api]);

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

  function toggleAgent(agent: AgentId) {
    setSelectedModes((current) => {
      const next = { ...current };
      if (next[agent] === "merge") delete next[agent];
      else next[agent] = "merge";
      return next;
    });
    setPreview(null);
    resetApprovals();
  }
  function toggleRebuild(agent: AgentId) {
    const state = byAgent.get(agent);
    if (!state?.invalid || !state.recovery?.eligible) return;
    setSelectedModes((current) => {
      const next = { ...current };
      if (next[agent] === "rebuild") delete next[agent];
      else next[agent] = "rebuild";
      return next;
    });
    setPreview(null);
    resetApprovals();
  }
  async function startOver(target: Stage = "select") {
    await destroyFlow();
    setDiscovery(null);
    setPreview(null);
    setResult(null);
    resetApprovals();
    setMessage("");
    setStage(target);
  }
  async function discover(event: React.FormEvent) {
    event.preventDefault();
    if (!selected.length) return;
    setBusy(true);
    setMessage("");
    setStage("discover");
    try {
      const value = await api.discoverModels(selected);
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
      const code = errorCode(error);
      setMessage(
        t(
          code === "MODEL_AUTH_FAILED"
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
        modes,
      );
      resetApprovals();
      setPreview(value);
      setConfig(value.model_config);
      setStage("preview");
    } catch (error) {
      const code = errorCode(error);
      if (code === "MODEL_CATALOG_STALE" || code === "MODEL_FLOW_EXPIRED") {
        await startOver("credential");
        setMessage(
          t(
            code === "MODEL_FLOW_EXPIRED"
              ? "agents.error.flowExpired"
              : "agents.error.catalogStale",
          ),
        );
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
  async function write(approveRebuild: AgentId[]) {
    if (!discovery || !preview || writeInFlightRef.current) return;
    writeInFlightRef.current = true;
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
        approveRebuild,
      );
      flowRef.current = "";
      setResult(value);
      setStage("result");
    } catch (error) {
      const code = errorCode(error);
      if (code === "PREVIEW_STALE") {
        await recoverStalePreview();
      } else if (code === "BACKUP_FAILED") {
        await destroyFlow();
        clearFlowState("select");
        setSelectedModes(detection ? initialSelection(detection) : {});
        setMessage(t("agents.error.backupFailed"));
      } else if (code === "WRITE_FAILED") {
        await destroyFlow();
        clearFlowState("select");
        setSelectedModes(detection ? initialSelection(detection) : {});
        setMessage(t("agents.error.rolledBack"));
      } else if (code === "ROLLBACK_FAILED") {
        await destroyFlow();
        await refreshDetection("select").catch(() => undefined);
        setMessage(t("agents.error.rollbackFailed"));
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
      writeInFlightRef.current = false;
      setBusy(false);
    }
  }
  async function finish() {
    setBusy(true);
    setMessage("");
    try {
      await refreshDetection("select");
    } catch {
      clearFlowState("select");
      setMessage(t("agents.error.detect"));
    } finally {
      setBusy(false);
    }
  }
  function confirmRebuild(agents: AgentId[]) {
    setRebuildDialogOpen(false);
    void write(agents);
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
  const selectedRebuildAgents = agentOrder.filter(
    (agent) => selectedModes[agent] === "rebuild",
  );
  const rebuildPreviewAgents = validateRebuildPreview(
    preview,
    selectedRebuildAgents,
  );
  const rebuildPreviewMatchesSelection = rebuildPreviewAgents !== null;
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
              const mode = selectedModes[id];
              const canRebuild = Boolean(
                agent?.invalid && agent.recovery?.eligible,
              );
              const reasons = agent ? recoveryReasons(agent) : [];
              return (
                <article
                  className={`agent-card${mode ? " is-selected" : ""}`}
                  key={id}
                >
                  <div className="agent-card__head">
                    <AgentLogo agent={id} />
                    <h3>{agentNames[id]}</h3>
                    <span className="agent-state">
                      {detectionState(agent, t)}
                    </span>
                  </div>
                  <p
                    className="agent-card__config-path"
                    title={agent ? safe(agent.path) : undefined}
                  >
                    {agent ? safe(agent.path) : t("agents.noResult")}
                  </p>
                  <p className="agent-card__guidance">
                    {detectionGuidance(agent, t)}
                  </p>
                  {agent?.invalid && (
                    <ul className="agent-recovery-reasons">
                      {(reasons.length ? reasons : [""]).map((reason) => (
                        <li key={reason || "unknown"}>
                          {recoveryReason(reason, t)}
                        </li>
                      ))}
                    </ul>
                  )}
                  <label className="agent-select">
                    <input
                      type="checkbox"
                      aria-label={t("agents.selectAgent", {
                        agent: agentNames[id],
                      })}
                      checked={mode === "merge"}
                      disabled={!agent || !selectable(agent)}
                      onChange={() => toggleAgent(id)}
                    />
                    <span>{t("agents.select")}</span>
                  </label>
                  {canRebuild && (
                    <button
                      type="button"
                      className={`agent-rebuild-toggle${mode === "rebuild" ? " is-active" : ""}`}
                      aria-pressed={mode === "rebuild"}
                      onClick={() => toggleRebuild(id)}
                    >
                      {t("agents.recovery.toggle", { agent: agentNames[id] })}
                    </button>
                  )}
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
          <div className="action-row">
            <button className="control-button" disabled={busy}>
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
              ref={rebuildTriggerRef}
              className="control-button"
              disabled={
                (preview.managed_config_drift && !approveDrift) ||
                (preview.requires_codex_auth_approval && !approveAuth) ||
                !rebuildPreviewMatchesSelection
              }
              onClick={() => {
                if (!rebuildPreviewAgents) return;
                if (rebuildPreviewAgents.length) setRebuildDialogOpen(true);
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
                  {selectedModes[agent.agent] === "rebuild" && (
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
      {rebuildDialogOpen &&
        rebuildPreviewAgents &&
        rebuildPreviewAgents.length > 0 && (
          <div
            className="dialog-backdrop"
            onMouseDown={(event) => {
              if (
                event.target === event.currentTarget &&
                !writeInFlightRef.current
              ) {
                closeRebuildDialog();
              }
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
                {rebuildPreviewAgents.map((agent) => (
                  <li key={agent}>{agentNames[agent]}</li>
                ))}
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
                  onClick={() => confirmRebuild(rebuildPreviewAgents)}
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
