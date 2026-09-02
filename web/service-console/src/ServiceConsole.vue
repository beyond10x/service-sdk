<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";

import type { JsonSchema, ServiceCatalog } from "./types";
import type { ServiceBinding } from "./binding";

interface Activity {
  id: number;
  operationRef: string;
  status: "running" | "succeeded" | "failed";
  output?: unknown;
  error?: string;
  replay?: boolean;
}

const props = withDefaults(
  defineProps<{
    catalog: ServiceCatalog;
    binding: ServiceBinding;
    mode?: "demo" | "live";
  }>(),
  { mode: "live" },
);

const selectedRef = ref(props.catalog.operations[0]?.operation_ref ?? "");
const values = reactive<Record<string, string>>({});
const confirmed = ref(false);
const activity = ref<Activity[]>([]);
const queryHistory = new Map<string, Record<string, unknown>>();
let activityId = 0;

const selected = computed(() =>
  props.catalog.operations.find(
    (operation) => operation.operation_ref === selectedRef.value,
  ),
);
const fields = computed(() =>
  Object.entries(selected.value?.input_schema.properties ?? {}),
);
const required = computed(
  () => new Set(selected.value?.input_schema.required ?? []),
);
const entities = computed(() => props.catalog.semantic_catalog.entities ?? []);
const views = computed(() => props.catalog.semantic_catalog.views ?? []);

watch(selectedRef, () => {
  for (const key of Object.keys(values)) delete values[key];
  confirmed.value = false;
});

async function submit(): Promise<void> {
  const operation = selected.value;
  if (!operation) return;
  if (operation.effect === "write" && !confirmed.value) return;
  const input = Object.fromEntries(
    fields.value
      .filter(([name]) => values[name] !== undefined && values[name] !== "")
      .map(([name, schema]) => [name, parseValue(values[name] ?? "", schema)]),
  );
  if (operation.kind === "query")
    queryHistory.set(operation.operation_ref, input);
  const succeeded = await invoke(
    operation.operation_ref,
    input,
    confirmed.value,
    false,
  );
  if (succeeded && operation.effect === "write") {
    for (const [operationRef, queryInput] of queryHistory) {
      await invoke(operationRef, queryInput, true, true);
    }
  }
  confirmed.value = false;
}

async function invoke(
  operationRef: string,
  input: Record<string, unknown>,
  isConfirmed: boolean,
  replay: boolean,
): Promise<boolean> {
  const entry = reactive<Activity>({
    id: ++activityId,
    operationRef,
    status: "running",
    replay,
  });
  activity.value.unshift(entry);
  try {
    const result = await props.binding.invoke(operationRef, input, {
      confirmed: isConfirmed,
    });
    entry.output = result.output;
    entry.status = "succeeded";
    return true;
  } catch (error) {
    entry.error = error instanceof Error ? error.message : String(error);
    entry.status = "failed";
    return false;
  }
}

function parseValue(value: string, schema: JsonSchema): unknown {
  const type = Array.isArray(schema.type)
    ? schema.type.find((candidate) => candidate !== "null")
    : schema.type;
  if (type === "integer" || type === "number") return Number(value);
  if (type === "boolean") return value === "true";
  if (type === "object" || type === "array") return JSON.parse(value);
  return value;
}

function controlType(schema: JsonSchema): string {
  if (schema.format === "date-time") return "datetime-local";
  const type = Array.isArray(schema.type) ? schema.type[0] : schema.type;
  return type === "integer" || type === "number" ? "number" : "text";
}
</script>

<template>
  <section class="service-console" :data-mode="mode">
    <header class="hero">
      <div>
        <p class="eyebrow">
          {{ mode === "demo" ? "Generated demo" : "Live service" }}
        </p>
        <h1>{{ catalog.display_name }}</h1>
        <p>{{ catalog.description }}</p>
      </div>
      <code>{{ catalog.service_ref }}</code>
    </header>

    <div v-if="mode === 'demo'" class="notice" role="status">
      Demo binding: validates the generated interaction surface but does not
      persist domain state.
    </div>

    <div class="console-grid">
      <nav aria-label="Service operations" class="operations">
        <button
          v-for="operation in catalog.operations"
          :key="operation.operation_ref"
          type="button"
          :class="{ selected: operation.operation_ref === selectedRef }"
          @click="selectedRef = operation.operation_ref"
        >
          <span>{{ operation.name }}</span>
          <small>{{ operation.kind }} · {{ operation.effect }}</small>
        </button>
      </nav>

      <form v-if="selected" class="operation-form" @submit.prevent="submit">
        <div>
          <p class="eyebrow">{{ selected.kind }}</p>
          <h2>{{ selected.name }}</h2>
          <code>{{ selected.semantic_ref }}</code>
        </div>

        <label v-for="[name, schema] in fields" :key="name">
          <span
            >{{ schema.title ?? name }}<b v-if="required.has(name)"> *</b></span
          >
          <small v-if="schema.description">{{ schema.description }}</small>
          <textarea
            v-if="schema.type === 'object' || schema.type === 'array'"
            v-model="values[name]"
            :required="required.has(name)"
            rows="4"
            placeholder="JSON"
          />
          <select
            v-else-if="schema.type === 'boolean'"
            v-model="values[name]"
            :required="required.has(name)"
          >
            <option value="">Select…</option>
            <option value="true">true</option>
            <option value="false">false</option>
          </select>
          <input
            v-else
            v-model="values[name]"
            :type="controlType(schema)"
            :required="required.has(name)"
          />
        </label>

        <label v-if="selected.effect === 'write'" class="confirmation">
          <input v-model="confirmed" type="checkbox" />
          <span>Confirm this state-changing intent</span>
        </label>
        <button
          class="run"
          type="submit"
          :disabled="selected.effect === 'write' && !confirmed"
        >
          {{ selected.kind === "query" ? "Run query" : "Send intent" }}
        </button>
      </form>
    </div>

    <div class="model-grid">
      <section>
        <h2>Lifecycle</h2>
        <article
          v-for="entity in entities"
          :key="entity.name"
          class="model-card"
        >
          <h3>{{ entity.display ?? entity.name }}</h3>
          <p>
            Initial state: <code>{{ entity.initial }}</code>
          </p>
          <ul>
            <li
              v-for="(transition, index) in entity.transitions ?? []"
              :key="transition.name ?? index"
            >
              {{
                transition.name ??
                `${transition.from?.join(", ") ?? "?"} → ${transition.to ?? "?"}`
              }}
            </li>
          </ul>
        </article>
        <p v-if="entities.length === 0" class="empty">
          No entity lifecycle is declared.
        </p>
      </section>
      <section>
        <h2>Views</h2>
        <article v-for="view in views" :key="view.name" class="model-card">
          <h3>{{ view.display ?? view.name }}</h3>
          <p>{{ view.consistency }}</p>
          <code>{{
            (view.fields ?? [])
              .map((field) => field.wire ?? field.name)
              .join(", ")
          }}</code>
        </article>
        <p v-if="views.length === 0" class="empty">No views are declared.</p>
      </section>
    </div>

    <section class="activity">
      <h2>Activity</h2>
      <p v-if="activity.length === 0" class="empty">
        Run an operation to see state changes and results.
      </p>
      <article
        v-for="entry in activity"
        :key="entry.id"
        :data-status="entry.status"
      >
        <div>
          <strong>{{ entry.operationRef }}</strong>
          <small>{{
            entry.replay ? "read-your-writes refresh" : entry.status
          }}</small>
        </div>
        <pre v-if="entry.output !== undefined">{{
          JSON.stringify(entry.output, null, 2)
        }}</pre>
        <p v-if="entry.error" role="alert">{{ entry.error }}</p>
      </article>
    </section>
  </section>
</template>

<style scoped>
.service-console {
  color: #14201d;
  background: #f5f3eb;
  border: 1px solid #d9d5c7;
  border-radius: 20px;
  overflow: hidden;
  font-family: Inter, ui-sans-serif, system-ui, sans-serif;
}
.hero {
  display: flex;
  justify-content: space-between;
  gap: 2rem;
  align-items: end;
  padding: 2rem;
  background: #173d34;
  color: #f8f4e7;
}
.hero h1 {
  margin: 0.2rem 0;
  font-size: clamp(2rem, 5vw, 3.5rem);
  line-height: 1;
}
.hero p {
  margin: 0.5rem 0 0;
  max-width: 55ch;
}
.hero code {
  color: #d7ef95;
}
.eyebrow {
  margin: 0;
  color: #6b8179;
  font-size: 0.75rem;
  font-weight: 800;
  letter-spacing: 0.12em;
  text-transform: uppercase;
}
.hero .eyebrow {
  color: #d7ef95;
}
.notice {
  padding: 0.8rem 2rem;
  background: #fff0b8;
  border-bottom: 1px solid #e0cc7d;
}
.console-grid {
  display: grid;
  grid-template-columns: minmax(12rem, 1fr) minmax(18rem, 2.5fr);
  min-height: 26rem;
}
.operations {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 1.25rem;
  background: #ebe8de;
  border-right: 1px solid #d9d5c7;
}
.operations button {
  display: grid;
  gap: 0.25rem;
  padding: 0.85rem;
  text-align: left;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: inherit;
  cursor: pointer;
}
.operations button.selected {
  background: #fff;
  border-color: #a7b6ae;
  box-shadow: 0 4px 16px #173d3412;
}
.operations small,
.operation-form small,
.activity small {
  color: #6b8179;
}
.operation-form {
  display: grid;
  align-content: start;
  gap: 1.1rem;
  padding: 2rem;
}
.operation-form h2 {
  margin: 0.25rem 0;
}
.operation-form label:not(.confirmation) {
  display: grid;
  gap: 0.35rem;
}
.operation-form input:not([type="checkbox"]),
.operation-form select,
.operation-form textarea {
  width: 100%;
  box-sizing: border-box;
  padding: 0.7rem 0.8rem;
  border: 1px solid #a7b6ae;
  border-radius: 8px;
  background: #fff;
  font: inherit;
}
.confirmation {
  display: flex;
  gap: 0.6rem;
  align-items: center;
}
.run {
  justify-self: start;
  padding: 0.75rem 1.1rem;
  border: 0;
  border-radius: 999px;
  background: #d7ef95;
  color: #173d34;
  font-weight: 800;
  cursor: pointer;
}
.run:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
.model-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1rem;
  padding: 1.5rem 2rem;
  border-top: 1px solid #d9d5c7;
}
.model-card {
  padding: 1rem;
  margin: 0.75rem 0;
  background: #fff;
  border-radius: 10px;
}
.model-card h3 {
  margin-top: 0;
}
.activity {
  padding: 1.5rem 2rem 2rem;
  border-top: 1px solid #d9d5c7;
}
.activity article {
  margin-top: 0.75rem;
  padding: 1rem;
  background: #fff;
  border-left: 4px solid #6b8179;
  border-radius: 8px;
}
.activity article[data-status="succeeded"] {
  border-color: #68a357;
}
.activity article[data-status="failed"] {
  border-color: #c05249;
}
.activity article > div {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
}
pre {
  overflow: auto;
  padding: 0.8rem;
  background: #14201d;
  color: #e7f4e5;
  border-radius: 6px;
}
.empty {
  color: #6b8179;
  font-style: italic;
}
@media (max-width: 760px) {
  .hero {
    align-items: start;
    flex-direction: column;
  }
  .console-grid,
  .model-grid {
    grid-template-columns: 1fr;
  }
  .operations {
    border-right: 0;
    border-bottom: 1px solid #d9d5c7;
  }
}
</style>
