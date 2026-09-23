//! Cross-artifact checks for the generated `/4` public mutation contract.

use std::{collections::BTreeMap, path::Path};

use service_builder::package::ServicePackageV4;

#[test]
fn billing_and_gatepass_catalogs_match_their_openapi_and_http_contract() {
    for service in ["billing", "gatepass"] {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../service-host/tests/fixtures/er-v4")
            .join(service);
        let package =
            ServicePackageV4::read(&fixture.join("package.yaml")).expect("strict /4 package reads");
        let build = service_builder::build_package_v4(&package).expect("/4 package generates");
        let artifacts = build.artifacts.iter().collect::<BTreeMap<_, _>>();
        let openapi: serde_json::Value = serde_json::from_str(artifacts["http/openapi.json"])
            .expect("generated OpenAPI is JSON");
        let receipt = &openapi["components"]["schemas"]["MutationReceipt"];
        let aftercare = &openapi["components"]["schemas"]["CommittedAftercare"];

        for operation in &build.service_catalog.operations {
            match operation.kind {
                service_catalog::CatalogOperationKind::Intent => {
                    assert_eq!(operation.output_schema["oneOf"][0], *receipt);
                    assert_eq!(operation.output_schema["oneOf"][1], *aftercare);
                    assert!(
                        openapi["paths"][format!("/v1/intents/{}", operation.name).as_str()]
                            ["post"]["responses"]
                            .get("202")
                            .is_some()
                    );
                }
                service_catalog::CatalogOperationKind::Query => {
                    assert!(
                        openapi["paths"][format!("/v1/queries/{}", operation.name).as_str()]
                            ["post"]["responses"]
                            .get("202")
                            .is_none()
                    );
                }
            }
        }
    }
}
