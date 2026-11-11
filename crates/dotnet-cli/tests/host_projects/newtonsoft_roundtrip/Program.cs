using System;
using Newtonsoft.Json;

public sealed class Payload
{
    public string name { get; set; }
    public int value { get; set; }
}

public static class Program
{
    public static int Main()
    {
        string json = JsonConvert.SerializeObject(new Payload { name = "test", value = 42 });
        Console.WriteLine(json);
        var result = JsonConvert.DeserializeObject<Payload>(json);
        return json == "{\"name\":\"test\",\"value\":42}" && result.name == "test" && result.value == 42 ? 42 : 1;
    }
}
