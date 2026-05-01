using System.Collections.Generic;
using System.Linq;

public static class Program
{
    private static IEnumerable<int> Values()
    {
        yield return 31;
        yield return 20;
        yield return 22;
        yield return 11;
        yield return 10;
        yield return 12;
    }

    public static int Main()
    {
        int[] expected = new[] { 10, 11, 12, 20, 22, 31 };

        int index = 0;
        foreach (int value in Values().OrderBy(item => item / 10).ThenBy(item => item % 10))
        {
            if (index >= expected.Length) return 1;
            if (value != expected[index]) return 2;
            index++;
        }

        if (index != expected.Length) return 3;
        return 42;
    }
}
