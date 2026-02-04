//! Example: Extract decorators from TypeScript code

use codegraph_extraction::tree_sitter_extractor::TreeSitterExtractor;
use codegraph_types::Language;

fn main() {
    let source = r#"
@LambdaFunction({
  directoryName: "guests-get-terms-and-conditions",
  memorySize: 512,
})
@IAMPolicyAccessDynamoDBTable(OrganizationsConfigurationEntriesEntity, {
  permissions: ["GetItem", "Query"],
})
@IAMPolicyAccessConfigParameters(PARAMETER_NAMES)
@AppSyncQuery({
  methodName: "guests_getTermsAndConditions",
  input: GuestsGetTermsAndConditionsInput,
  output: GuestsGetTermsAndConditionsResultType,
  enablePublicWithApiKey: true,
})
export class GuestsGetTermsAndConditionsService {
    handler = async (input: any): Promise<any> => {
        return {};
    };
}
"#;

    let mut extractor = TreeSitterExtractor::new();
    let result = extractor.extract(source, "test.ts", Language::TypeScript).unwrap();

    println!("=== Extracted Nodes ===");
    for node in &result.nodes {
        println!("\nNode: {} ({})", node.name, node.kind.as_str());
        if !node.decorators.is_empty() {
            println!("  Decorators:");
            for dec in &node.decorators {
                println!("    - {}", dec);
            }
        }
        println!("  Exported: {}", node.is_exported);
    }
}
