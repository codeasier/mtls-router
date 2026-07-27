import {
  forwardRef,
  useEffect,
  useEffectEvent,
  useId,
  useImperativeHandle,
  useRef,
  useState,
  type ComponentPropsWithoutRef,
  type ReactNode,
} from "react";

import type { AgentTarget } from "./agentPresentation";
import { agentNames } from "./agentPresentation";
import { useI18n } from "./i18n";
import {
  sanitizeSensitiveText,
  type AgentId,
  type AgentModelsResult,
  type JsonObject,
  type ModelConfig,
  type ModelSelection,
  type OpenCodeModelConfig,
} from "./ipc";

const roleNames = ["haiku", "sonnet", "opus"] as const;

function safe(value: string | undefined) {
  return sanitizeSensitiveText(value ?? "");
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
  resetToken,
  onChange,
  onErrorChange,
}: {
  label: string;
  value?: JsonObject;
  resetToken: number;
  onChange(value?: JsonObject): void;
  onErrorChange(error?: string): void;
}) {
  const { t } = useI18n();
  const errorId = useId();
  const serialized = value ? JSON.stringify(value, null, 2) : "";
  const [text, setText] = useState(serialized);
  const locallyEmitted = useRef<string | null>(null);
  const previousResetToken = useRef(resetToken);
  const reportError = useEffectEvent(onErrorChange);
  useEffect(() => {
    if (previousResetToken.current !== resetToken) {
      previousResetToken.current = resetToken;
      setText(serialized);
      reportError(undefined);
      locallyEmitted.current = null;
      return;
    }
    if (serialized === locallyEmitted.current) {
      locallyEmitted.current = null;
      return;
    }
    setText(serialized);
    reportError(undefined);
  }, [resetToken, serialized]);
  useEffect(() => () => reportError(undefined), []);
  const parsed = parseExtra(text);
  return (
    <div className="object-field">
      <label>
        <span>{label}</span>
        <textarea
          aria-invalid={Boolean(parsed.error)}
          aria-describedby={errorId}
          value={text}
          placeholder={t("agents.unset")}
          onChange={(event) => {
            const next = event.target.value;
            setText(next);
            const result = parseExtra(next);
            onErrorChange(result.error);
            if (!next.trim()) {
              locallyEmitted.current = "";
              onChange(undefined);
            } else if (result.value) {
              locallyEmitted.current = JSON.stringify(result.value, null, 2);
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
      <small id={errorId} role={parsed.error ? "alert" : undefined}>
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
      <ul className="catalog-list" aria-label={t("agents.catalogLabel")}>
        {visible.map((model) => (
          <li className="catalog-model" key={model}>
            <code>{safe(model)}</code>
          </li>
        ))}
        {!visible.length && <li>{t("agents.catalogEmptySearch")}</li>}
      </ul>
    </div>
  );
}

function OpenCodeSettings({
  model,
  settings,
  resetToken,
  update,
  onFieldError,
}: {
  model: string;
  settings: OpenCodeModelConfig;
  resetToken: number;
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
          resetToken={resetToken}
          onChange={(value) => update({ ...settings, options: value })}
          onErrorChange={(error) => onFieldError("options", error)}
        />
        <ObjectField
          label={t("agents.variantsJson")}
          value={settings.variants}
          resetToken={resetToken}
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
          resetToken={resetToken}
          onChange={(value) => update({ ...settings, extra: value })}
          onErrorChange={(error) => onFieldError("extra", error)}
        />
      </div>
    </details>
  );
}

// Shared by the workflow and preview boundary to keep fail-closed validation singular.
// eslint-disable-next-line react-refresh/only-export-components
export function validateSingleTargetConfig(
  config: ModelConfig,
  target: AgentId,
) {
  return (
    config[target] !== undefined &&
    (["claude", "opencode", "codex"] as AgentId[]).every(
      (agent) => agent === target || config[agent] === undefined,
    )
  );
}

function configError(
  config: ModelConfig,
  target: AgentId,
  extra: string,
  objectFieldErrors: Record<string, string>,
) {
  if (!validateSingleTargetConfig(config, target))
    return "/: single_agent_required";
  if (target === "claude" && config.claude) {
    if (!config.claude.primary.model) return "/claude/primary/model: required";
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
    target === "opencode" &&
    config.opencode &&
    (!Object.keys(config.opencode.models).length ||
      !config.opencode.default_model)
  )
    return "/opencode/default_model: required";
  if (target === "codex" && config.codex && !config.codex.model)
    return "/codex/model: required";
  const parsed = parseExtra(extra);
  if (parsed.error) return `/${target}/extra${parsed.error}`;
  const objectError = Object.entries(objectFieldErrors)[0];
  if (objectError)
    return `/opencode/models/${objectError[0]}: ${objectError[1]}`;
  return "";
}

function withExtra(config: ModelConfig, target: AgentId, extra: string) {
  const value = structuredClone(config);
  const parsed = parseExtra(extra).value;
  if (target === "claude" && value.claude) delete value.claude.extra;
  if (target === "codex" && value.codex) delete value.codex.extra;
  if (parsed && Object.keys(parsed).length) {
    if (target === "claude" && value.claude)
      value.claude.extra = Object.fromEntries(
        Object.entries(parsed).map(([key, child]) => [key, String(child)]),
      );
    if (target === "codex" && value.codex)
      value.codex.extra = parsed as NonNullable<typeof value.codex.extra>;
  }
  return value;
}

function serializedExtra(config: ModelConfig, target: AgentId) {
  const extra =
    target === "claude"
      ? config.claude?.extra
      : target === "codex"
        ? config.codex?.extra
        : undefined;
  return extra && Object.keys(extra).length
    ? JSON.stringify(extra, null, 2)
    : "";
}

// Allows hosts to render the initial disabled state before the field ref mounts.
// eslint-disable-next-line react-refresh/only-export-components
export function validateAgentConfig(config: ModelConfig, target: AgentId) {
  return configError(config, target, serializedExtra(config, target), {});
}

export interface AgentConfigDraftState {
  error: string;
  hasLocalDraft: boolean;
}

export interface AgentConfigSnapshot extends AgentConfigDraftState {
  config: ModelConfig;
}

export interface AgentConfigFieldsHandle {
  getSnapshot(): AgentConfigSnapshot;
}

export interface AgentConfigFieldsProps {
  target: AgentTarget;
  discovery: AgentModelsResult;
  config: ModelConfig;
  disabled: boolean;
  resetToken: number;
  beforeFields?: ReactNode;
  afterFields?: ReactNode;
  onChange(config: ModelConfig): void;
  onDraftStateChange(state: AgentConfigDraftState): void;
}

export const AgentConfigFields = forwardRef<
  AgentConfigFieldsHandle,
  AgentConfigFieldsProps
>(function AgentConfigFields(
  {
    target,
    discovery,
    config,
    disabled,
    resetToken,
    beforeFields,
    afterFields,
    onChange,
    onDraftStateChange,
  },
  ref,
) {
  const { t } = useI18n();
  const [search, setSearch] = useState("");
  const [extra, setExtra] = useState(() =>
    serializedExtra(config, target.agent),
  );
  const [objectFieldErrors, setObjectFieldErrors] = useState<
    Record<string, string>
  >({});
  const configRef = useRef(config);
  const extraRef = useRef(extra);
  const objectFieldErrorsRef = useRef(objectFieldErrors);
  const previousResetToken = useRef(resetToken);
  const reportDraftState = useEffectEvent(onDraftStateChange);
  const invalid = configError(config, target.agent, extra, objectFieldErrors);

  useEffect(() => {
    configRef.current = config;
  }, [config]);
  useEffect(() => {
    if (previousResetToken.current === resetToken) return;
    previousResetToken.current = resetToken;
    const nextExtra = serializedExtra(config, target.agent);
    extraRef.current = nextExtra;
    objectFieldErrorsRef.current = {};
    setExtra(nextExtra);
    setObjectFieldErrors({});
  }, [config, resetToken, target.agent]);

  function snapshot(
    currentConfig = configRef.current,
    currentExtra = extraRef.current,
    currentObjectErrors = objectFieldErrorsRef.current,
  ): AgentConfigSnapshot {
    const error = configError(
      currentConfig,
      target.agent,
      currentExtra,
      currentObjectErrors,
    );
    return {
      config: error
        ? currentConfig
        : withExtra(currentConfig, target.agent, currentExtra),
      error,
      hasLocalDraft:
        Boolean(parseExtra(currentExtra).error) ||
        Object.keys(currentObjectErrors).length > 0,
    };
  }

  useImperativeHandle(ref, () => ({ getSnapshot: snapshot }));
  useEffect(
    () =>
      reportDraftState({
        error: invalid,
        hasLocalDraft:
          Boolean(parseExtra(extra).error) ||
          Object.keys(objectFieldErrors).length > 0,
      }),
    [extra, invalid, objectFieldErrors],
  );
  useEffect(
    () => () => reportDraftState({ error: "", hasLocalDraft: false }),
    [],
  );

  function emitConfig(next: ModelConfig) {
    const parsed = parseExtra(extraRef.current);
    const value = parsed.error
      ? next
      : withExtra(next, target.agent, extraRef.current);
    configRef.current = value;
    onChange(value);
    reportDraftState(snapshot(value));
  }

  function updateCodexExtra(next: ModelConfig) {
    const nextExtra = serializedExtra(next, "codex");
    extraRef.current = nextExtra;
    configRef.current = next;
    setExtra(nextExtra);
    onChange(next);
    reportDraftState(snapshot(next, nextExtra));
  }

  function updateExtra(next: string) {
    extraRef.current = next;
    setExtra(next);
    if (!parseExtra(next).error) {
      const value = withExtra(configRef.current, target.agent, next);
      configRef.current = value;
      onChange(value);
    }
    reportDraftState(snapshot(configRef.current, next));
  }

  function setObjectFieldError(path: string, error?: string) {
    const next = { ...objectFieldErrorsRef.current };
    if (error) next[path] = error;
    else delete next[path];
    objectFieldErrorsRef.current = next;
    setObjectFieldErrors(next);
    reportDraftState(snapshot(configRef.current, extraRef.current, next));
  }

  return (
    <>
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
        {beforeFields}
        <fieldset
          className="agent-config-fields"
          disabled={disabled}
          style={{ border: 0, margin: 0, minWidth: 0, padding: 0 }}
        >
          {target.agent === "claude" && config.claude && (
            <section className="model-agent-panel">
              <h3>Claude Code</h3>
              <ClaudeSelectionFields
                id="claude-primary"
                selection={config.claude.primary}
                models={discovery.models}
                modelLabel={t("agents.primaryModel")}
                onChange={(primary) =>
                  emitConfig({
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
                    emitConfig({
                      ...config,
                      claude: { ...config.claude!, context_window },
                    })
                  }
                />
                <OptionalNumber
                  label={t("agents.claudeMaxOutputTokens")}
                  value={config.claude.max_output_tokens}
                  onChange={(max_output_tokens) =>
                    emitConfig({
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
                        emitConfig({
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
                        emitConfig({
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
                      emitConfig({ ...config, claude });
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
                          emitConfig({
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
                          emitConfig({
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
                value={extra}
                onChange={updateExtra}
              />
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
                          emitConfig({
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
                    emitConfig({
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
                    resetToken={resetToken}
                    onFieldError={(field, error) =>
                      setObjectFieldError(`${model}/${field}`, error)
                    }
                    update={(next) =>
                      emitConfig({
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
                    emitConfig({
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
                    emitConfig({
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
                    emitConfig({
                      ...config,
                      codex: {
                        ...config.codex!,
                        reasoning_summary: reasoning_summary as
                          "auto" | "concise" | "detailed" | "none" | undefined,
                      },
                    })
                  }
                />
                <OptionalSelect
                  label={t("agents.verbosity")}
                  value={config.codex.verbosity}
                  values={["low", "medium", "high"]}
                  onChange={(verbosity) =>
                    emitConfig({
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
                    emitConfig({
                      ...config,
                      codex: { ...config.codex!, context_window },
                    })
                  }
                />
                <OptionalNumber
                  label={t("agents.compactLimit")}
                  value={config.codex.auto_compact_token_limit}
                  onChange={(auto_compact_token_limit) =>
                    emitConfig({
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
                    updateCodexExtra({
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
              <ExtraEditor agent="codex" value={extra} onChange={updateExtra} />
            </section>
          )}
        </fieldset>
        {afterFields}
      </div>
    </>
  );
});
