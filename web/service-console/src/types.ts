export type RealmPolicy = 'required' | 'optional' | 'forbidden'
export type CatalogOperationKind = 'intent' | 'query'
export type CatalogOperationEffect = 'read' | 'write'

export interface JsonSchema {
  type?: string | string[]
  title?: string
  description?: string
  format?: string
  properties?: Record<string, JsonSchema>
  required?: string[]
  items?: JsonSchema
  enum?: unknown[]
  anyOf?: JsonSchema[]
  additionalProperties?: boolean
  [key: string]: unknown
}

export interface CatalogOperation {
  name: string
  operation_ref: string
  semantic_ref: string
  kind: CatalogOperationKind
  effect: CatalogOperationEffect
  input_schema: JsonSchema
  output_schema: JsonSchema
}

export interface EssEntity {
  name: string
  display?: string
  initial: string
  states?: string[]
  transitions?: Array<{ name?: string; from?: string[]; to?: string }>
  views?: string[]
}

export interface EssView {
  name: string
  display?: string
  fields?: Array<{ name: string; wire?: string; spelling?: string }>
  consistency?: string
}

export interface EssBrowserCatalog {
  format: 'ess-browser-catalog/1'
  system?: string
  display?: string
  summary?: string
  commands?: Array<Record<string, unknown>>
  views?: EssView[]
  entities?: EssEntity[]
  events?: Array<Record<string, unknown>>
  [key: string]: unknown
}

export interface ServiceCatalog {
  format: 'service-catalog/1'
  service_ref: string
  display_name: string
  description: string
  semantic_catalog: EssBrowserCatalog
  authentication: {
    source: 'session'
    realm_policy: RealmPolicy
  }
  operations: CatalogOperation[]
}

const authenticationCoordinates = new Set([
  'tenant',
  'tenant_id',
  'tenantid',
  'realm',
  'realm_id',
  'realmid',
  'user',
  'user_id',
  'userid',
  'authority',
  'authority_id',
  'authorityid',
  'principal',
  'principal_id',
  'principalid',
  'executor',
  'executor_id',
  'executorid',
])

export function assertServiceCatalog(value: unknown): asserts value is ServiceCatalog {
  if (typeof value !== 'object' || value === null) throw new Error('service catalog must be an object')
  const catalog = value as Partial<ServiceCatalog>
  if (
    catalog.format !== 'service-catalog/1' ||
    catalog.semantic_catalog?.format !== 'ess-browser-catalog/1' ||
    catalog.authentication?.source !== 'session' ||
    !Array.isArray(catalog.operations)
  ) {
    throw new Error('unsupported service catalog')
  }
  const seen = new Set<string>()
  for (const operation of catalog.operations) {
    if (!operation.operation_ref || seen.has(operation.operation_ref)) {
      throw new Error(`duplicate service operation: ${operation.operation_ref}`)
    }
    seen.add(operation.operation_ref)
    for (const input of Object.keys(operation.input_schema.properties ?? {})) {
      if (authenticationCoordinates.has(input.toLowerCase())) {
        throw new Error(`authentication coordinate is not an operation input: ${input}`)
      }
    }
  }
}
