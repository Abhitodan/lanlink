package com.lanlink.app.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.lanlink.app.ui.MainViewModel
import com.lanlink.app.ui.PairState
import com.lanlink.app.ui.Screen
import com.lanlink.app.ui.theme.Glyphs

@Composable
fun PairScreen(vm: MainViewModel, s: Screen.Pair) {
    val state by vm.pair.collectAsState()
    var address by rememberSaveable { mutableStateOf(s.prefillAddress) }
    var port by rememberSaveable { mutableStateOf(s.prefillPort.toString()) }
    var code by rememberSaveable { mutableStateOf("") }
    var manual by rememberSaveable { mutableStateOf(s.prefillAddress.isNotEmpty()) }
    val scanner = rememberLauncherForActivityResult(ScanContract()) { r ->
        r.contents?.let { vm.pairFromUri(it) }
    }
    DisposableEffect(Unit) { onDispose { if (vm.pair.value !is PairState.Done) vm.cancelPairing() } }

    Page("Pair a PC", onBack = { vm.cancelPairing(); vm.back() }) {
        when (val st = state) {
            PairState.Idle, is PairState.Failed -> {
                if (st is PairState.Failed) {
                    Card {
                        Text("Pairing did not complete", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.error)
                        Note(st.message)
                    }
                }
                Note("On the PC, open LANLink and choose \"Pair a phone\". A QR code and an 8-digit code appear for two minutes.")
                PrimaryButton("Scan QR code", {
                    scanner.launch(
                        ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                            .setPrompt("Point at the QR code on your PC").setBeepEnabled(false).setOrientationLocked(false),
                    )
                }, icon = Glyphs.QrScan)
                if (!manual) {
                    SecondaryButton("Enter the code instead", { manual = true })
                } else {
                    SectionLabel("Enter manually")
                    OutlinedTextField(
                        address, { address = it.trim() }, Modifier.fillMaxWidth(),
                        label = { Text("PC address (for example 192.168.1.20)") }, singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedTextField(
                            code, { v -> code = v.filter { it.isDigit() }.take(8) }, Modifier.weight(2f),
                            label = { Text("8-digit code") }, singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                        )
                        OutlinedTextField(
                            port, { v -> port = v.filter { it.isDigit() }.take(5) }, Modifier.weight(1f),
                            label = { Text("Port") }, singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        )
                    }
                    PrimaryButton(
                        "Pair",
                        { vm.pairManual(address, port.toIntOrNull() ?: 47990, code) },
                        enabled = address.isNotBlank() && code.length == 8,
                    )
                    Note("Without the QR code, the phone checks the PC's identity using the code itself and the confirmation number shown next.")
                }
            }
            is PairState.Offer -> {
                Text("Pair with ${st.name.ifEmpty { "this PC" }}?", style = MaterialTheme.typography.headlineMedium)
                Note("A pairing link was opened. Continue only if you just chose \"Pair a phone\" on your own PC.")
                Card {
                    Text("Address: ${st.addresses.joinToString(", ")}", style = MaterialTheme.typography.bodyMedium)
                    Text("PC identity: ${st.fingerprint.take(16).chunked(4).joinToString(" ")}", style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace)
                }
                PrimaryButton("Pair", { vm.acceptOffer() })
                SecondaryButton("Cancel", { vm.cancelPairing(); vm.back() })
            }
            is PairState.Working -> CenteredProgress(st.stage)
            is PairState.Confirm -> {
                Text("Check the PC", style = MaterialTheme.typography.headlineMedium)
                Note("The PC is asking whether to trust this phone. Approve it only if the PC shows this same number:")
                Text(
                    st.code.chunked(3).joinToString(" "),
                    style = MaterialTheme.typography.headlineMedium.copy(fontSize = 44.sp, fontFamily = FontFamily.Monospace),
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Confirmation number ${st.code.toList().joinToString(" ")}" },
                )
                CenteredProgress("Waiting for approval on the PC…")
                SecondaryButton("Cancel", { vm.cancelPairing() })
            }
            is PairState.Done -> {
                Text("Paired with ${st.host.name}", style = MaterialTheme.typography.headlineMedium)
                Note("This phone can now connect to the PC whenever both are on the same network.")
                PrimaryButton("Connect now", {
                    vm.cancelPairing()
                    vm.replace(Screen.Home)
                    vm.connect(st.host)
                }, icon = Glyphs.Monitor)
                SecondaryButton("Done", { vm.cancelPairing(); vm.replace(Screen.Home) })
            }
        }
    }
}
