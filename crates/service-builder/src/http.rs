//! Deterministic `OpenAPI` projection for the generated Identity HTTP boundary.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use service_definition::ServiceDelivery;

use crate::client::{ClientInput, ClientOperationKind, ClientPlan, ClientResult};

/// Emits `OpenAPI` only when the service selected Identity HTTP delivery.
#[allow(clippy::too_many_lines)]
pub fn openapi(client: &ClientPlan) -> Option<String> {
    let ServiceDelivery::IdentityHttp { audience } = &client.delivery else {
        return None;
    };
    let mut paths = BTreeMap::new();
    paths.insert(
        "/healthz".to_owned(),
        json!({"get": {"operationId": "health", "responses": {"204": {"description": "healthy"}}}}),
    );
    paths.insert(
        "/readyz".to_owned(),
        json!({"get": {"operationId": "ready", "responses": {"204": {"description": "ready"}}}}),
    );
    for operation in &client.operations {
        let prefix = match operation.kind {
            ClientOperationKind::Intent => "intents",
            ClientOperationKind::Query => "queries",
        };
        let mut body_schema = object_schema(&operation.inputs);
        if operation.kind == ClientOperationKind::Query {
            body_schema
                .as_object_mut()
                .and_then(|schema| schema.get_mut("properties"))
                .and_then(Value::as_object_mut)
                .expect("generated request object has properties")
                .insert(
                    "$page".to_owned(),
                    json!({
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "cursor": {"type": ["string", "null"]},
                            "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
                        },
                        "required": ["limit"]
                    }),
                );
        }
        let success_schema = match &operation.result {
            ClientResult::Intent { .. } => json!({"$ref": "#/components/schemas/MutationReceipt"}),
            ClientResult::Query { fields } => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "items": {"type": "array", "items": object_schema_from_fields(fields)},
                    "through_version": {"type": ["integer", "null"], "minimum": 0},
                    "next_cursor": {"type": ["string", "null"]},
                    "partial": {"type": "boolean"}
                },
                "required": ["items", "through_version", "next_cursor", "partial"]
            }),
        };
        paths.insert(
            format!("/v1/{prefix}/{}", operation.operation),
            json!({
                "post": {
                    "operationId": operation.operation,
                    "x-b10x-semantic-ref": operation.semantic_ref,
                    "x-b10x-scope": operation.scope,
                    "security": [{"identityBearer": [operation.scope.clone()]}],
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": body_schema}}
                    },
                    "responses": {
                        "200": {"description": "accepted", "content": {"application/json": {"schema": success_schema}}},
                        "400": problem_response("invalid input"),
                        "401": problem_response("unauthorized"),
                        "403": problem_response("forbidden"),
                        "409": problem_response("service invariant refused the operation"),
                        "503": problem_response("service unavailable")
                    }
                }
            }),
        );
    }
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": format!("{} generated service", client.service), "version": "1"},
        "x-b10x-audience": audience,
        "paths": paths,
        "components": {
            "securitySchemes": {
                "identityBearer": {"type": "http", "scheme": "bearer", "bearerFormat": "opaque"}
            },
            "schemas": {
                "Problem": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"code": {"type": "string"}, "status": {"type": "integer"}, "detail": {"type": "string"}},
                    "required": ["code", "status", "detail"]
                },
                "MutationReceipt": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "outcome": {"type": "string"},
                        "events": {"type": "array", "items": {"type": "object"}},
                        "through_version": {"type": "integer", "minimum": 0},
                        "replayed": {"type": "boolean"}
                    },
                    "required": ["outcome", "events", "through_version", "replayed"]
                }
            }
        }
    });
    let mut output =
        serde_json::to_string_pretty(&document).expect("generated OpenAPI document serializes");
    output.push('\n');
    Some(output)
}

fn object_schema(inputs: &[ClientInput]) -> Value {
    let fields = inputs
        .iter()
        .map(|input| (input.name.clone(), input.type_ref.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut schema = object_schema_from_fields(&fields);
    let required = inputs
        .iter()
        .filter(|input| !input.optional)
        .map(|input| Value::String(input.name.clone()))
        .collect::<Vec<_>>();
    schema
        .as_object_mut()
        .expect("object schema is an object")
        .insert("required".to_owned(), Value::Array(required));
    schema
}

fn object_schema_from_fields(fields: &BTreeMap<String, String>) -> Value {
    let properties = fields
        .iter()
        .map(|(name, type_ref)| (name.clone(), type_schema(type_ref)))
        .collect::<BTreeMap<_, _>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties
    })
}

fn type_schema(type_ref: &str) -> Value {
    if let Some(inner) = type_ref
        .strip_prefix("Optional<")
        .and_then(|value| value.strip_suffix('>'))
    {
        let mut schema = type_schema(inner);
        schema
            .as_object_mut()
            .expect("type schema is an object")
            .insert("nullable".to_owned(), Value::Bool(true));
        return schema;
    }
    if let Some(inner) = type_ref
        .strip_prefix("List<")
        .and_then(|value| value.strip_suffix('>'))
    {
        return json!({"type": "array", "items": type_schema(inner)});
    }
    match type_ref {
        "Integer" => json!({"type": "integer"}),
        "Decimal" => json!({"type": "number"}),
        "Boolean" => json!({"type": "boolean"}),
        "Uuid" => json!({"type": "string", "format": "uuid"}),
        "String" | "Timestamp" | "DateTime" => json!({"type": "string"}),
        _ => json!({}),
    }
}

fn problem_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": {"application/problem+json": {"schema": {"$ref": "#/components/schemas/Problem"}}}
    })
}
