using System.Collections.Generic;

public static class Program
{
    private static int _finallyRuns;

    public static IEnumerable<int> Numbers()
    {
        try
        {
            yield return 10;
            yield return 20;
            yield return 30;
        }
        finally
        {
            _finallyRuns += 7;
        }
    }

    public static int Main()
    {
        int sum = 0;

        foreach (int value in Numbers())
        {
            sum += value;
            break;
        }

        return sum == 10 && _finallyRuns == 7 ? 42 : 1;
    }
}
