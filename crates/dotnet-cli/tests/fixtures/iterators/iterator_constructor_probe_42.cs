using System.Collections.Generic;

public static class Program
{
    public static IEnumerable<int> Numbers()
    {
        yield return 40;
        yield return 2;
    }

    public static int Main()
    {
        IEnumerable<int> sequence = Numbers();
        IEnumerator<int> enumerator = sequence.GetEnumerator();
        if (enumerator == null)
        {
            return 1;
        }

        if (!enumerator.MoveNext())
        {
            return 2;
        }

        return enumerator.Current == 40 ? 42 : 3;
    }
}
