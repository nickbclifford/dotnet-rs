using System.Text.Json;

public static class Program
{
    public static int Main()
    {
        // Parse a JSON object and verify property extraction
        string json = "{\"Name\":\"Alice\",\"Age\":30}";
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;

        if (root.ValueKind != JsonValueKind.Object) return 1;
        if (root.GetProperty("Name").GetString() != "Alice") return 2;
        if (root.GetProperty("Age").GetInt32() != 30) return 3;

        // Parse a JSON array and verify indexed access
        string arrayJson = "[10, 20, 30]";
        using var arrayDoc = JsonDocument.Parse(arrayJson);
        var arr = arrayDoc.RootElement;

        if (arr.ValueKind != JsonValueKind.Array) return 4;
        if (arr.GetArrayLength() != 3) return 5;
        if (arr[1].GetInt32() != 20) return 6;

        return 42;
    }
}
