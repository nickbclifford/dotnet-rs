using System.Collections.Generic;
using System.Linq;

public static class Program
{
    public static int Main()
    {
        var source = new List<int>(512);
        for (int i = 0; i < 512; i++)
        {
            source.Add(i);
        }

        int checksum = 0;

        for (int round = 0; round < 50; round++)
        {
            var evens = new List<int>(
                source
                    .Where(value => (value & 1) == 0)
                    .Select(value => value + 1)
            );

            if (evens.Count != 256 || evens[0] != 1 || evens[255] != 511)
            {
                return 1;
            }

            int skip = round % 17;
            var window = new List<int>(evens.Skip(skip).Take(64));
            if (window.Count != 64)
            {
                return 2;
            }

            int expectedFirst = 1 + (2 * skip);
            int expectedLast = expectedFirst + 126;
            if (window[0] != expectedFirst || window[63] != expectedLast)
            {
                return 3;
            }

            var distinct = new List<int>(
                source
                    .Select(value => value % 37)
                    .Distinct()
            );

            if (distinct.Count != 37 || distinct[0] != 0 || distinct[36] != 36)
            {
                return 4;
            }

            checksum += window[0] + window[63] + distinct[round % 37];
        }

        return checksum == 8712 ? 42 : 5;
    }
}
