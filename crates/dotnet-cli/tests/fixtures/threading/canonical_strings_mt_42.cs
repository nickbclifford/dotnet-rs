using System;
using System.Runtime.CompilerServices;
using System.Threading;

// A worker that materializes a literal or an empty string first must not own
// the canonical object. Main observes the same identity after the worker has exited.
static class Program
{
    static string? workerLiteral, workerEmpty;
    static string?[] workerEmpties = new string?[7];
    static ConditionalWeakTable<string, object> table = new();

    static void Worker()
    {
        workerLiteral = "canonical worker-first literal 42";
        workerEmpty = string.Empty;
        char[] none = new char[0];
        workerEmpties[0] = new string(none);
        workerEmpties[1] = new string(none, 0, 0);
        workerEmpties[2] = new string('x', 0);
        workerEmpties[3] = new string(ReadOnlySpan<char>.Empty);
        workerEmpties[4] = "abc".Substring(1, 0);
        workerEmpties[5] = "abc".Remove(0);
        workerEmpties[6] = "";
        table.Add(workerLiteral, new object());
    }

    static int Main()
    {
        var worker = new Thread(Worker);
        worker.Start();
        worker.Join();

        bool literal = ReferenceEquals(workerLiteral, "canonical worker-first literal 42")
            && ReferenceEquals(string.IsInterned("canonical worker-first literal 42"), workerLiteral)
            && table.TryGetValue("canonical worker-first literal 42", out _);
        bool empty = ReferenceEquals(workerEmpty, string.Empty);
        bool empties = true;
        for (int i = 0; i < workerEmpties.Length; i++)
            if (!ReferenceEquals(workerEmpties[i], string.Empty)) { empties = false; Console.WriteLine("empty form " + i + " differs"); }

        var weak = new WeakReference(workerLiteral);
        workerLiteral = workerEmpty = null;
        for (int i = 0; i < workerEmpties.Length; i++) workerEmpties[i] = null;
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        bool survives = ReferenceEquals(weak.Target, "canonical worker-first literal 42")
            && table.TryGetValue("canonical worker-first literal 42", out _);

        Console.WriteLine("literal: " + literal);
        Console.WriteLine("empty: " + empty);
        Console.WriteLine("empty forms: " + empties);
        Console.WriteLine("survives: " + survives);
        return literal && empty && empties && survives ? 42 : 1;
    }
}
