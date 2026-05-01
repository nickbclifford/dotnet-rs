using System.Collections.Generic;
using System.Linq;

public static class Program
{
    private static IEnumerable<int> Values()
    {
        yield return 9;
        yield return 8;
        yield return 7;
        yield return 6;
        yield return 5;
    }

    public static int Main()
    {
        int[] expected = new[] { 8, 7, 6 };
        int index = 0;
        int sum = 0;

        foreach (int value in Values().Skip(1).Take(3))
        {
            if (index >= expected.Length) return 1;
            if (value != expected[index]) return 2;
            sum += value;
            index++;
        }

        if (index != expected.Length) return 3;
        if (sum != 21) return 4;
        return 42;
    }
}
