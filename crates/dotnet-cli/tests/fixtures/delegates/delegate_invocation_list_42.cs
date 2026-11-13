using System;

delegate int UserDelegate();

class Program
{
    static int First() => 1;
    static int Second() => 2;

    static void CheckTypes(object value)
    {
        Console.WriteLine($"{value is MulticastDelegate} {value is Delegate}");
    }

    static void CheckList(Delegate value)
    {
        Delegate[] entries = value.GetInvocationList();
        Console.Write(entries.Length);
        foreach (Delegate entry in entries)
            Console.Write($" {((UserDelegate)entry)()}");
        Console.WriteLine();
        entries[0] = null!;
        Console.WriteLine(value.GetInvocationList()[0] != null);
    }

    static int Main()
    {
        UserDelegate first = First;
        UserDelegate combined = first + Second;
        CheckList(first);
        CheckList(combined);
        CheckTypes(first);
        CheckTypes(combined);
        CheckTypes((Func<int>)First);
        CheckTypes((Action)(() => { }));
        CheckTypes(new object());
        return 42;
    }
}
