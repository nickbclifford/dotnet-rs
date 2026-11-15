using System;
using System.Text.Json;

// System.Text.Json reading numbers back and writing literals. Odd-length property names
// ("Score", "Zip") used to be skipped because the VM's string hash read past the characters, and
// writing true/false/null segfaulted on the u8-literal copy (plan 24 slice 3 gaps).
public class Address { public string City { get; set; } = ""; public int Zip { get; set; } }
public class Person
{
    public string Name { get; set; } = "";
    public double Score { get; set; }
    public int Id { get; set; }
    public bool Active { get; set; }
    public string? Note { get; set; }
    public Address Home { get; set; } = new();
}

public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }

    public static int Main()
    {
        const string json =
            "{\"Name\":\"n\",\"Score\":1.5,\"Id\":3,\"Active\":true,\"Note\":null,\"Home\":{\"City\":\"London\",\"Zip\":1815}}";
        var p = JsonSerializer.Deserialize<Person>(json)!;
        Check(1, p.Name == "n");
        Check(2, p.Score == 1.5);
        Check(3, p.Id == 3);
        Check(4, p.Active);
        Check(5, p.Note == null);
        Check(6, p.Home.City == "London");
        Check(7, p.Home.Zip == 1815);

        Check(8, JsonSerializer.Serialize(p) == json);
        Check(9, JsonSerializer.Serialize(true) == "true");
        Check(10, JsonSerializer.Serialize((int?)null) == "null");
        Check(11, JsonSerializer.Deserialize<double>("2.25") == 2.25);

        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}
