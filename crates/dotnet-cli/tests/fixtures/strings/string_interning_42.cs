using System;

public static class Program
{
    static string LiteralFromAnotherMethod() => "literal identity";
    static string LoopLiteral() => "loop identity";

    static void Report(string name, bool result) => Console.WriteLine(name + ": " + result);
    static void ReportException(string name, Exception error) =>
        Console.WriteLine(name + ": throws " + error.GetType().FullName);

    public static unsafe int Main()
    {
        string first = "literal identity";
        string second = "literal identity";
        Report("literal within method", ReferenceEquals(first, second));
        Report("literal across methods", ReferenceEquals(first, LiteralFromAnotherMethod()));

        string previous = LoopLiteral();
        bool loopIdentity = true;
        for (int i = 0; i < 3; i++)
        {
            string current = LoopLiteral();
            loopIdentity &= ReferenceEquals(previous, current);
            previous = current;
        }
        Report("literal in loop", loopIdentity);

        Report("empty field", ReferenceEquals(string.Empty, string.Empty));
        Report("empty literal", ReferenceEquals(string.Empty, ""));
        Report("utf16 literal", ReferenceEquals("\ud800\0\udfff", Utf16Literal()));
        Report("computed is distinct", !ReferenceEquals(first, new string(new[] { 'l', 'i', 't', 'e', 'r', 'a', 'l', ' ', 'i', 'd', 'e', 'n', 't', 'i', 't', 'y' })));
        WeakReference literalRoot = LiteralWeakReference();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        Report("literal after collection", ReferenceEquals(first, LiteralFromAnotherMethod()));
        Report("empty after collection", ReferenceEquals(string.Empty, ""));
        Report("intern table roots literal", ReferenceEquals(literalRoot.Target, RootedLiteral()));
        Report("null pointer constructor", ReferenceEquals(new string((char*)null), string.Empty));
        Report("zero length constructor", ReferenceEquals(new string('a', 0), string.Empty));

        string computedLiteral = new string(new[] { 'l', 'i', 't', 'e', 'r', 'a', 'l', ' ', 'i', 'd', 'e', 'n', 't', 'i', 't', 'y' });
        try { Report("intern literal", ReferenceEquals(string.Intern(computedLiteral), first)); }
        catch (Exception error) { ReportException("intern literal", error); }

        // This content is never loaded with ldstr, so it starts outside the intern table.
        string computed = new string(new[] { 'u', 'n', 'i', 'q', 'u', 'e', '-', '4', '2' });
        try { Report("is interned before", string.IsInterned(computed) == null); }
        catch (Exception error) { ReportException("is interned before", error); }
        try
        {
            string canonical = string.Intern(computed);
            string equalComputed = new string(new[] { 'u', 'n', 'i', 'q', 'u', 'e', '-', '4', '2' });
            Report("is interned after", ReferenceEquals(string.IsInterned(equalComputed), canonical));
        }
        catch (Exception error) { ReportException("is interned after", error); }
        return 42;
    }

    static string Utf16Literal() => "\ud800\0\udfff";
    static string RootedLiteral() => "intern table root 42";
    static WeakReference LiteralWeakReference() => new WeakReference(RootedLiteral());
}
