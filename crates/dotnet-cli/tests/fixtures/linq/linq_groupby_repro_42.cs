using System.Collections.Generic;
using System.Linq;

public static class Program
{
    private static IEnumerable<int> Values()
    {
        yield return 1;
        yield return 2;
        yield return 3;
        yield return 4;
        yield return 5;
        yield return 6;
    }

    public static int Main()
    {
        int sum0 = 0;
        int sum1 = 0;
        int sum2 = 0;
        int groups = 0;

        foreach (var group in Values().GroupBy(value => value % 3))
        {
            groups++;
            int total = 0;
            foreach (int value in group)
            {
                total += value;
            }

            if (group.Key == 0) sum0 = total;
            else if (group.Key == 1) sum1 = total;
            else if (group.Key == 2) sum2 = total;
            else return 1;
        }

        if (groups != 3) return 2;
        if (sum0 != 9 || sum1 != 5 || sum2 != 7) return 3;
        return 42;
    }
}
