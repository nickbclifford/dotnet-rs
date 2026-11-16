using System;
using System.Collections.Generic;
using System.Text.Json;

public static class Program
{
    private sealed class Address
    {
        public string City { get; set; }
        public int Zip { get; set; }
    }

    private sealed class Person
    {
        public int Id { get; set; }
        public string Name { get; set; }
        public Address Home { get; set; }
        public List<int> Tags { get; set; }
        public int? Optional { get; set; }
        public double Score { get; set; }
    }

    private static int Fail(int code)
    {
        Console.WriteLine("FAIL " + code);
        return code;
    }

    public static int Main()
    {
        // System.Text.Json picks its Reflection.Emit accessor from IsDynamicCodeSupported. bool and null
        // values are left out because the writer's "true"u8 / "null"u8 constants crash the VM, and
        // numbers are only written, never read back, because the reader sometimes returns 0 for a
        // number (all separate gaps).
        Person person = new Person
        {
            Id = 7,
            Name = "Ada",
            Score = 1.5,
            Home = new Address { City = "London", Zip = 1815 },
            Tags = new List<int> { 1, 2, 3 },
            Optional = 5,
        };

        string json = JsonSerializer.Serialize(person);
        Console.WriteLine(json);
        if (json != "{\"Id\":7,\"Name\":\"Ada\",\"Home\":{\"City\":\"London\",\"Zip\":1815},\"Tags\":[1,2,3],\"Optional\":5,\"Score\":1.5}") return Fail(1);

        Person parsed = JsonSerializer.Deserialize<Person>("{\"Name\":\"Bob\",\"Home\":{\"City\":\"Paris\"}}");
        if (parsed.Name != "Bob" || parsed.Home.City != "Paris") return Fail(2);

        Address address = JsonSerializer.Deserialize<Address>("{\"City\":\"Rome\"}");
        if (address.City != "Rome") return Fail(3);

        Console.WriteLine("ok");
        return 42;
    }
}
