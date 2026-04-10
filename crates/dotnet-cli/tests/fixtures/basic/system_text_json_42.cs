using System.Text.Json;

public static class Program
{
    public static int Main()
    {
        var options = new JsonSerializerOptions
        {
            PropertyNameCaseInsensitive = true,
            WriteIndented = false
        };

        if (!options.PropertyNameCaseInsensitive) return 1;
        if (options.WriteIndented) return 2;

        return 42;
    }
}
