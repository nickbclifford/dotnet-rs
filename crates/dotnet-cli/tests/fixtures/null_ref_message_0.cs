using System;

class Program
{
    static int Main()
    {
        try
        {
            object obj = null;
            obj.ToString(); // Should throw NullReferenceException
            return 1; // Should not reach here
        }
        catch (NullReferenceException ex)
        {
            if (ex.Message != null && ex.Message.Length > 0 && ex.Message == "Object reference not set to an instance of an object.")
            {
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
