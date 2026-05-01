using System;
using System.Collections.Generic;
using System.Linq;

public static class Program
{
    private static IEnumerable<int> Values()
    {
        yield return 5;
        yield return 3;
        yield return 1;
        yield return 3;
        yield return 2;
        yield return 4;
    }

    private static IEnumerable<int> ZipValues()
    {
        yield return 10;
        yield return 20;
        yield return 30;
    }

    public static int Main()
    {
        int[] distinct = Values().Distinct().ToArray();
        if (distinct.Length != 5) return 1;
        if (distinct[0] != 5 || distinct[1] != 3 || distinct[2] != 1 || distinct[3] != 2 || distinct[4] != 4) return 2;

        int skipTakeSum = Values().Skip(1).Take(3).Sum();
        if (skipTakeSum != 7) return 3;

        int zipSum = Values()
            .Take(3)
            .Zip(ZipValues(), (left, right) => left + right)
            .Sum();
        if (zipSum != 69) return 4;

        int orderedChecksum = Values()
            .Select((value, index) => value * 10 + index)
            .OrderBy(item => item / 10)
            .ThenBy(item => item % 10)
            .Sum();
        if (orderedChecksum != 195) return 5;

        int[] groupedSums = Values()
            .GroupBy(value => value % 2)
            .OrderBy(group => group.Key)
            .Select(group => group.Sum())
            .ToArray();
        if (groupedSums.Length != 2 || groupedSums[0] != 6 || groupedSums[1] != 12) return 6;

        int sideEffects = 0;
        IEnumerable<int> lazy = Values().Select(value =>
        {
            sideEffects++;
            return value * 2;
        });

        if (sideEffects != 0) return 7;
        int[] lazyMaterialized = lazy.Take(2).ToArray();
        if (sideEffects != 2) return 8;
        if (lazyMaterialized.Length != 2 || lazyMaterialized[0] != 10 || lazyMaterialized[1] != 6) return 9;

        return 42;
    }
}
