using System.Collections.Generic;
using System.Linq;

public static class Program
{
    public static int Main()
    {
        List<int> values = new List<int> { 2, 4, 6 };

        int aggregate = values.Aggregate((left, right) => left + right);

        int sum = values
            .Where(value => value > 0)
            .Select(value => value)
            .Sum();

        var list = values.ToList();

        int[] array = values.ToArray();

        var dictionary = values
            .Where(value => value >= 4)
            .ToDictionary(value => value, value => value + 1);

        if (aggregate != 12) return 2;
        if (sum != 12) return 3;
        if (list.Count != 3) return 4;
        if (array == null) return 5;
        if (dictionary.Count != 2 || dictionary[4] != 5 || dictionary[6] != 7) return 6;

        return 42;
    }
}
