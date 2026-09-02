import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import { ServiceConsole, assertServiceCatalog, createDemoServiceBinding, type ServiceCatalog } from '../src'

const catalog: ServiceCatalog = {
  format: 'service-catalog/1',
  service_ref: 'service:todo',
  display_name: 'Todo',
  description: 'Generated Todo service',
  semantic_catalog: {
    format: 'ess-browser-catalog/1',
    entities: [{ name: 'todo.list.List', initial: 'Active', transitions: [] }],
    views: [{ name: 'todo.list.ListById', consistency: 'read_your_writes', fields: [{ name: 'list_id' }] }],
  },
  authentication: { source: 'session', realm_policy: 'optional' },
  operations: [
    {
      name: 'create_list',
      operation_ref: 'todo.create_list',
      semantic_ref: 'todo.list.CreateList',
      kind: 'intent',
      effect: 'write',
      input_schema: {
        type: 'object',
        properties: { list_id: { type: 'string' } },
        required: ['list_id'],
        additionalProperties: false,
      },
      output_schema: { type: 'object' },
    },
    {
      name: 'get_list',
      operation_ref: 'todo.get_list',
      semantic_ref: 'todo.list.ListById',
      kind: 'query',
      effect: 'read',
      input_schema: {
        type: 'object',
        properties: { list_id: { type: 'string' } },
        required: ['list_id'],
        additionalProperties: false,
      },
      output_schema: { type: 'array' },
    },
  ],
}

describe('ServiceConsole', () => {
  it('requires confirmation before sending a generated write intent', async () => {
    const binding = { invoke: vi.fn().mockResolvedValue({ output: { outcome: 'created' } }) }
    const wrapper = mount(ServiceConsole, { props: { catalog, binding } })
    await wrapper.get('input[type="text"]').setValue('list-1')
    expect(wrapper.get('button.run').attributes('disabled')).toBeDefined()
    await wrapper.get('input[type="checkbox"]').setValue(true)
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(binding.invoke).toHaveBeenCalledWith('todo.create_list', { list_id: 'list-1' }, { confirmed: true })
  })

  it('ships an explicit non-persistent demo binding', async () => {
    const demo = createDemoServiceBinding(catalog)
    await expect(demo.invoke('todo.get_list', { list_id: 'list-1' }, { confirmed: true })).resolves.toEqual({
      output: [],
      connector_audit_ref: 'demo:1',
    })
  })

  it('refuses authentication coordinates in a catalog form', () => {
    const invalid = structuredClone(catalog) as unknown as Record<string, unknown>
    const operations = invalid.operations as Array<Record<string, unknown>>
    const schema = operations[0]?.input_schema as Record<string, unknown>
    schema.properties = { realm: { type: 'string' } }
    expect(() => assertServiceCatalog(invalid)).toThrow('authentication coordinate')
  })
})
