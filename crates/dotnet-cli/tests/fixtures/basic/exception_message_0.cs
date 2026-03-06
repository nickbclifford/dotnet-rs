using System;

class Program
{
    static int Main()
    {
        try
        {
            int[] arr = new int[1];
            int x = arr[5]; // Should throw IndexOutOfRangeException
            return 1; // Should not reach here
        }
        catch (IndexOutOfRangeException ex)
        {
            if (ex.Message != null && ex.Message.Length > 0 && ex.Message == "Index was outside the bounds of the array.")
            {
                // Verify it's actually the message we set in instructions/memory/arrays.rs or similar
                return 0; // Success
            }
            return 2; // Message is null, empty, or wrong
        }
        catch (Exception)
        {
            return 3; // Wrong exception type
        }
    }
}
