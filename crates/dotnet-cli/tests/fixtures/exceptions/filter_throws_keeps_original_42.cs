using System;

public class Program {
    static int filterCalls;

    static bool ThrowFromFilter() {
        filterCalls++;
        throw new ArgumentException("filter exception");
    }

    public static int Main() {
        var original = new InvalidOperationException("original exception");
        try {
            try {
                throw original;
            } catch (InvalidOperationException) when (ThrowFromFilter()) {
                return 1;
            }
        } catch (InvalidOperationException caught) {
            return ReferenceEquals(caught, original) && filterCalls == 1 ? 42 : 2;
        } catch (Exception) {
            return 3;
        }
        return 4;
    }
}
