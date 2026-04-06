using System;

public delegate int MyDelegate(int x);

public class Program {
    static int trace = 0;

    public static int First(int x) {
        trace = trace * 10 + 1;
        return x + 1;
    }

    public static int Second(int x) {
        trace = trace * 10 + 2;
        return x + 2;
    }

    public static int Main() {
        MyDelegate first = First;
        MyDelegate second = Second;

        MyDelegate chain = (MyDelegate)Delegate.Combine(first, second);
        chain = (MyDelegate)Delegate.Combine(chain, first); // [First, Second, First]

        int result = chain(10);
        if (result != 11) return 1;
        if (trace != 121) return 2;

        trace = 0;
        MyDelegate removedLast = (MyDelegate)Delegate.Remove(chain, first); // [First, Second]
        if (removedLast == null) return 3;
        if (removedLast(10) != 12) return 4;
        if (trace != 12) return 5;

        trace = 0;
        MyDelegate tailPattern = (MyDelegate)Delegate.Combine(second, first);
        MyDelegate removedTail = (MyDelegate)Delegate.Remove(chain, tailPattern); // [First]
        if (removedTail == null) return 6;
        if (removedTail(10) != 11) return 7;
        if (trace != 1) return 8;

        return 42;
    }
}
