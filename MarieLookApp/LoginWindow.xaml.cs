using System.Windows;

namespace MarieLookApp;

public partial class LoginWindow : Window
{
    private string _email = string.Empty;

    public event Action? LoginSuccessful;

    public LoginWindow()
    {
        InitializeComponent();
        Loaded += (s, e) => EmailBox.Focus();
    }

    private async void OnSendOtpClick(object sender, RoutedEventArgs e)
    {
        _email = EmailBox.Text.Trim();

        if (string.IsNullOrEmpty(_email) || !_email.Contains('@'))
        {
            StatusText.Text = "Please enter a valid email address.";
            return;
        }

        SendOtpButton.IsEnabled = false;
        StatusText.Text = "";
        StatusText.Foreground = System.Windows.Media.Brushes.Gray;
        StatusText.Text = "Sending code...";

        var result = await App.Supabase.SendOtpAsync(_email);

        if (result.Success)
        {
            EmailPanel.Visibility = Visibility.Collapsed;
            OtpPanel.Visibility = Visibility.Visible;
            StatusText.Text = "";
            OtpBox.Focus();
        }
        else
        {
            StatusText.Foreground = System.Windows.Media.Brushes.Red;
            StatusText.Text = result.Error ?? "Failed to send code.";
            SendOtpButton.IsEnabled = true;
        }
    }

    private async void OnVerifyClick(object sender, RoutedEventArgs e)
    {
        var otp = OtpBox.Text.Trim();

        if (string.IsNullOrEmpty(otp))
        {
            StatusText.Text = "Please enter the code.";
            return;
        }

        VerifyButton.IsEnabled = false;
        StatusText.Foreground = System.Windows.Media.Brushes.Gray;
        StatusText.Text = "Verifying...";

        var result = await App.Supabase.VerifyOtpAsync(_email, otp);

        if (result.Success)
        {
            LoginSuccessful?.Invoke();
            Close();
        }
        else
        {
            StatusText.Foreground = System.Windows.Media.Brushes.Red;
            StatusText.Text = result.Error ?? "Invalid code.";
            VerifyButton.IsEnabled = true;
        }
    }

    private void OnBackClick(object sender, RoutedEventArgs e)
    {
        OtpPanel.Visibility = Visibility.Collapsed;
        EmailPanel.Visibility = Visibility.Visible;
        SendOtpButton.IsEnabled = true;
        StatusText.Text = "";
        OtpBox.Text = "";
    }
}
