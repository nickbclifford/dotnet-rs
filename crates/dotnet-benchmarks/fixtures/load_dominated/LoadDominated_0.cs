using System.Text.Json;

// Forces loading the full framework working set (System.Text.Json and its transitive deps:
// System.Runtime, System.Collections, System.Linq, etc.) while executing almost nothing.
// Cold minus warm ≈ pure metadata-load cost. Used as the P1 regression guard.
class Program {
    static int Main() {
        using var doc = JsonDocument.Parse("0");
        return doc.RootElement.GetInt32();
    }
}
