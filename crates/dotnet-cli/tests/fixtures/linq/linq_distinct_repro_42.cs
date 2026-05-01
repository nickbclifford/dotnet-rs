using System.Collections.Generic;
using System.Linq;

public static class Program
{
    private static IEnumerable<int> Values()
    {
        yield return 5;
        yield return 3;
        yield return 5;
        yield return 2;
        yield return 3;
        yield return 2;
        yield return 5;
    }

    public static int Main()
    {
        int[] expected = new[] { 5, 3, 2 };

        int index = 0;
        foreach (int value in Values().Distinct())
        {
            if (index >= expected.Length) return 1;
            if (value != expected[index]) return 2;
            index++;
        }

        if (index != expected.Length) return 3;
        return 42;
    }
}
