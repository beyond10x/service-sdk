import type { CatalogOperation, ServiceCatalog } from './types'

export interface ServiceInvocation {
  output: unknown
  connector_audit_ref?: string
}

export interface InvokeOptions {
  confirmed: boolean
}

/** Product-owned invocation seam. Authentication stays in the hosting session. */
export interface ServiceBinding {
  invoke(operationRef: string, input: Record<string, unknown>, options: InvokeOptions): Promise<ServiceInvocation>
}

export interface HttpServiceBindingOptions {
  endpoint: string
  fetch?: typeof globalThis.fetch
}

/** Live binding for a product BFF which keeps Connector leases and tokens server-side. */
export function createHttpServiceBinding(options: HttpServiceBindingOptions): ServiceBinding {
  const fetcher = options.fetch ?? globalThis.fetch
  return {
    async invoke(operationRef, input, invocation) {
      const response = await fetcher(options.endpoint, {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ operation_ref: operationRef, input, confirmed: invocation.confirmed }),
      })
      if (!response.ok) throw new Error(`service invocation failed: ${response.status}`)
      return (await response.json()) as ServiceInvocation
    },
  }
}

/** Explicitly disposable standalone-docs binding; it never claims to persist domain state. */
export function createDemoServiceBinding(catalog: ServiceCatalog): ServiceBinding {
  const operations = new Map(catalog.operations.map((operation) => [operation.operation_ref, operation]))
  let sequence = 0
  return {
    async invoke(operationRef, input, options) {
      const operation = operations.get(operationRef)
      if (!operation) throw new Error(`unknown demo operation: ${operationRef}`)
      if (operation.effect === 'write' && !options.confirmed) throw new Error('write confirmation is required')
      sequence += 1
      return {
        output: demoOutput(operation, input, sequence),
        connector_audit_ref: `demo:${sequence}`,
      }
    },
  }
}

function demoOutput(operation: CatalogOperation, input: Record<string, unknown>, sequence: number): unknown {
  if (operation.kind === 'query') return []
  return {
    outcome: 'demo_only',
    events: [],
    through_version: sequence,
    replayed: false,
    submitted_input: input,
  }
}
