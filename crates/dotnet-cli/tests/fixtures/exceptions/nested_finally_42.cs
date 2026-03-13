using System;

public class Program {
    public static int Main() {
        try {
            try {
                throw new InvalidOperationException("First");
            } finally {
                // This exception should replace "First"
                throw new Exception("Second");
            }
        } catch (Exception e) {
            if (e.Message == "Second") return 42;
        }
        return 0;
    }
}
