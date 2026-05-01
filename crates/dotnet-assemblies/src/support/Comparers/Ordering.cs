using System.Collections.Generic;

namespace DotnetRs.Comparers;

public class Ordering
{
    // implementations adapted from System.Private.CoreLib comparer behavior
    public class FallbackComparer<T> : Comparer<T>
    {
        public override int Compare(T? x, T? y)
        {
            if (x is null)
            {
                return y is null ? 0 : -1;
            }

            if (y is null)
            {
                return 1;
            }

            if (x is IComparable<T> genericComparable)
            {
                return genericComparable.CompareTo(y);
            }

            if (x is IComparable comparable)
            {
                return comparable.CompareTo(y);
            }

            throw new ArgumentException("At least one object must implement IComparable.");
        }
    }
}
