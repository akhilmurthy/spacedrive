import { CompositeScreenProps } from '@react-navigation/native';
import { createStackNavigator, StackScreenProps } from '@react-navigation/stack';
import Header from '~/components/header/Header';
import { tw } from '~/lib/tailwind';
import NetworkScreen from '~/screens/network';

import { TabScreenProps } from '../TabNavigator';

const Stack = createStackNavigator<NetworkStackParamList>();

export default function NetworkStack() {
	return (
		<Stack.Navigator
			initialRouteName="Network"
			screenOptions={{
				headerStyle: { backgroundColor: tw.color('app-box') },
				headerTintColor: tw.color('ink'),
				headerTitleStyle: tw`text-base`,
				headerBackTitleStyle: tw`text-base`
			}}
		>
			<Stack.Screen name="Network" component={NetworkScreen} options={{ header: Header }} />
		</Stack.Navigator>
	);
}

export type NetworkStackParamList = {
	Network: undefined;
};

export type NetworkStackScreenProps<Screen extends keyof NetworkStackParamList> =
	CompositeScreenProps<
		StackScreenProps<NetworkStackParamList, Screen>,
		TabScreenProps<'NetworkStack'>
	>;
