using System;
using System.Threading;

static class Program
{
    static string? literalA, literalB, emptyA, emptyB;
    static string? inputA, inputB, internA, internB;
    static int ready, published;
    static bool exchangeA, exchangeB;

    static string RaceInput() => new string(new[] { 'r', 'a', 'c', 'e', '-', '4', '2', '\ud800', '\0' });

    static void Worker(bool first)
    {
        string literal = "cross-thread literal 42";
        string empty = string.Empty;
        string input = RaceInput();
        if (first) { literalA = literal; emptyA = empty; inputA = input; }
        else { literalB = literal; emptyB = empty; inputB = input; }

        Interlocked.Increment(ref ready);
        while (Volatile.Read(ref ready) != 2) Thread.Sleep(0);
        string interned = string.Intern(input);
        if (first) internA = interned; else internB = interned;

        Interlocked.Increment(ref published);
        while (Volatile.Read(ref published) != 2) Thread.Sleep(0);
        bool exchanged = ReferenceEquals(literal, first ? literalB : literalA)
            && ReferenceEquals(empty, first ? emptyB : emptyA)
            && ReferenceEquals(interned, first ? internB : internA);
        if (first) exchangeA = exchanged; else exchangeB = exchanged;
    }

    static int Main()
    {
        var a = new Thread(() => Worker(true));
        var b = new Thread(() => Worker(false));
        a.Start(); b.Start(); a.Join(); b.Join();

        bool literals = ReferenceEquals(literalA, literalB) && ReferenceEquals(literalA, "cross-thread literal 42");
        bool empty = ReferenceEquals(emptyA, emptyB) && ReferenceEquals(emptyA, string.Empty)
            && ReferenceEquals(emptyA, "");
        bool race = !ReferenceEquals(inputA, inputB) && ReferenceEquals(internA, internB)
            && (ReferenceEquals(internA, inputA) || ReferenceEquals(internA, inputB))
            && ReferenceEquals(string.IsInterned(RaceInput()), internA);
        var literalRoot = new WeakReference(literalA);
        var internRoot = new WeakReference(internA);
        var winningInputRoot = new WeakReference(ReferenceEquals(internA, inputA) ? inputA : inputB);
        literalA = literalB = emptyA = emptyB = inputA = inputB = internA = internB = null;
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        bool rooted = ReferenceEquals(literalRoot.Target, "cross-thread literal 42")
            && internRoot.Target != null && ReferenceEquals(internRoot.Target, string.IsInterned(RaceInput()))
            && ReferenceEquals(winningInputRoot.Target, internRoot.Target);

        Console.WriteLine("literal: " + literals);
        Console.WriteLine("empty: " + empty);
        Console.WriteLine("exchange: " + (exchangeA && exchangeB));
        Console.WriteLine("intern race: " + race);
        Console.WriteLine("retired arena roots: " + rooted);
        return literals && empty && exchangeA && exchangeB && race && rooted ? 42 : 1;
    }
}
