using System.Text.Json;

public static class Program
{
    private static readonly string[] Payloads =
    {
        "{\"id\":\"eleven\",\"values\":{\"a\":\"twelve\",\"b\":\"thirteen\",\"c\":\"fourteen\"}}",
        "{\"id\":\"twenty-one\",\"values\":{\"a\":\"twenty-two\",\"b\":\"twenty-three\",\"c\":\"twenty-four\"}}",
        "{\"id\":\"thirty-one\",\"values\":{\"a\":\"thirty-two\",\"b\":\"thirty-three\",\"c\":\"thirty-four\"}}",
        "{\"id\":\"forty-one\",\"values\":{\"a\":\"forty-two\",\"b\":\"forty-three\",\"c\":\"forty-four\"}}",
        "{\"id\":\"fifty-one\",\"values\":{\"a\":\"fifty-two\",\"b\":\"fifty-three\",\"c\":\"fifty-four\"}}",
    };

    public static int Main()
    {
        var checksum = 0;

        for (var i = 0; i < 60; i++)
        {
            using var document = JsonDocument.Parse(Payloads[i % Payloads.Length]);
            var root = document.RootElement;
            var values = root.GetProperty("values");

            checksum += root.GetProperty("id").GetString()!.Length;
            checksum += values.GetProperty("a").GetString()!.Length;
            checksum += values.GetProperty("b").GetString()!.Length;
            checksum += values.GetProperty("c").GetString()!.Length;
        }

        return checksum > 0 ? 0 : 1;
    }
}
