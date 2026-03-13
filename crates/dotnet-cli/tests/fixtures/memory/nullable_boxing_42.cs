using System;

public class Program {
    public static int Main() {
        int? n1 = null;
        int? n2 = 42;

        object b1 = n1;
        object b2 = n2;

        if (b1 != null) {
            Console.WriteLine("FAIL: boxed null Nullable<int> is not null");
            return 1;
        }

        if (b2 == null) {
            Console.WriteLine("FAIL: boxed non-null Nullable<int> is null");
            return 2;
        }

        if (!(b2 is int)) {
            Console.WriteLine("FAIL: boxed non-null Nullable<int> is not int: " + b2.GetType().FullName);
            return 3;
        }

        int val = (int)b2;
        if (val != 42) {
            Console.WriteLine("FAIL: boxed non-null Nullable<int> value is " + val);
            return 4;
        }

        int? n3 = (int?)b2;
        if (!n3.HasValue || n3.Value != 42) {
            Console.WriteLine("FAIL: unboxed Nullable<int> from boxed int is incorrect");
            return 5;
        }

        int? n4 = (int?)b1;
        if (n4.HasValue) {
            Console.WriteLine("FAIL: unboxed Nullable<int> from null is not null");
            return 6;
        }

        Console.WriteLine("SUCCESS");
        return 42;
    }
}
