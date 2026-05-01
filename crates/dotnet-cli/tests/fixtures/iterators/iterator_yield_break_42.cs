using System.Collections.Generic;

public static class Program
{
    public static IEnumerable<int> Numbers(bool stopEarly)
    {
        yield return 40;

        if (stopEarly)
        {
            yield break;
        }

        yield return 2;
    }

    public static int Main()
    {
        int earlyCount = 0;
        int earlySum = 0;
        foreach (int value in Numbers(true))
        {
            earlyCount++;
            earlySum += value;
        }

        if (earlyCount != 1 || earlySum != 40)
        {
            return 1;
        }

        int fullCount = 0;
        int fullSum = 0;
        foreach (int value in Numbers(false))
        {
            fullCount++;
            fullSum += value;
        }

        return fullCount == 2 && fullSum == 42 ? 42 : 2;
    }
}
