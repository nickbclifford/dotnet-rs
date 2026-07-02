using System.Collections.Generic;

namespace DotnetRs.Comparers;

public class Ordering
{
    // implementations adapted from System.Private.CoreLib comparer behavior
    public class FallbackComparer<T> : Comparer<T>
    {
        public override int Compare(T? x, T? y)
        {
            if (ReferenceEquals(x, y))
            {
                return 0;
            }

            if (x is null)
            {
                return -1;
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

            if (y is IComparable reverseComparable)
            {
                return -reverseComparable.CompareTo(x);
            }

            throw new ArgumentException("At least one object must implement IComparable.");
        }
    }
}
